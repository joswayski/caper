//! A virtualised list of variable-height rows: the channel history lays out
//! only the rows near its viewport and replaces the rest with spacers.
//!
//! Each row's height is measured whenever it is drawn and cached by its key
//! together with a revision of what affects its layout (width, text size,
//! edits, reactions, grouping, dividers…). A row that was never drawn gets
//! the running mean of measured rows once, and keeps that estimate until it
//! is measured, so later measurements elsewhere never move it.
//!
//! Scroll anchoring keeps what the reader sees still:
//! - After each pass, the first row in the viewport and where its bottom was
//!   are remembered. The next pass recomputes that row's position from the
//!   (possibly updated) heights and moves the offset by the difference, so
//!   older history prepended, or rows above measured or changed, never move
//!   the rows being read.
//! - Rows above that anchor are drawn only to be measured: at their estimated
//!   tops, and clipped above the viewport, so a wrong estimate there never
//!   pushes visible rows; it becomes the next pass's correction instead.
//! - If the anchor row itself turns out a different height (its header went
//!   away because it now groups under a prepended row, an image loaded), the
//!   pass asks to be redone with its bottom kept in place.
//!
//! At the live end the scroll area's own stick-to-bottom keeps the newest row
//! in place instead. A jump centres its target and keeps re-centring it as
//! nearby rows are measured, until it settles, the reader scrolls, or a short
//! deadline passes.

use std::collections::HashMap;
use std::ops::Range;

/// Laid out beyond each edge of the viewport, so rows are usually measured
/// before they scroll into view.
pub const OVERSCAN: f32 = 600.0;
/// A row's height before anything has been measured.
const DEFAULT_HEIGHT: f32 = 60.0;
/// Seconds a jump keeps re-centring its target.
const JUMP_SETTLE: f64 = 1.5;
/// Position differences below this are rounding, not movement.
const EPSILON: f32 = 0.5;
/// Heights kept for rows no longer in the list (other conversations).
const RETAINED: usize = 4_000;

/// One row: a stable key and a hash of everything that affects its layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug)]
struct Height {
    height: f32,
    /// The revision it was measured at; `None` for an estimate.
    revision: Option<u64>,
}

/// Measured (or once-estimated) row heights.
#[derive(Default)]
pub struct Heights {
    rows: HashMap<String, Height>,
    measured_sum: f64,
    measured: usize,
}

impl Heights {
    /// The mean measured height, in whole points: the scroll area snaps
    /// its content to pixels, so a fractional estimate above the reader
    /// would leave what they read a fraction of a pixel off.
    fn estimate(&self) -> f32 {
        if self.measured == 0 {
            DEFAULT_HEIGHT
        } else {
            (self.measured_sum / self.measured as f64).round() as f32
        }
    }

    /// The height to lay `row` out with: its last measurement (even at an
    /// older revision, the best guess until it is drawn again), else an
    /// estimate fixed the first time it is asked for.
    fn get(&mut self, row: &Row) -> f32 {
        if let Some(height) = self.rows.get(&row.key) {
            return height.height;
        }
        let height = self.estimate();
        self.rows.insert(
            row.key.clone(),
            Height {
                height,
                revision: None,
            },
        );
        height
    }

    /// Records a drawn row's height; true when it differs from the height
    /// it was laid out with.
    fn measure(&mut self, row: &Row, height: f32) -> bool {
        let previous = self.rows.insert(
            row.key.clone(),
            Height {
                height,
                revision: Some(row.revision),
            },
        );
        match previous {
            Some(Height {
                height: old,
                revision: Some(_),
            }) => {
                self.measured_sum += f64::from(height - old);
                (height - old).abs() > EPSILON
            }
            other => {
                self.measured_sum += f64::from(height);
                self.measured += 1;
                other.is_none_or(|old| (height - old.height).abs() > EPSILON)
            }
        }
    }

    /// Forgets rows that are not in `rows` once many have built up.
    fn prune(&mut self, rows: &[Row]) {
        if self.rows.len() <= rows.len() + RETAINED {
            return;
        }
        let keep: std::collections::HashSet<&str> =
            rows.iter().map(|row| row.key.as_str()).collect();
        self.rows.retain(|key, _| keep.contains(key.as_str()));
        self.measured = self
            .rows
            .values()
            .filter(|height| height.revision.is_some())
            .count();
        self.measured_sum = self
            .rows
            .values()
            .filter(|height| height.revision.is_some())
            .map(|height| f64::from(height.height))
            .sum();
    }
}

/// Row tops from the start of the list (`tops[len]` is the total height).
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    tops: Vec<f32>,
}

impl Layout {
    fn new(rows: &[Row], heights: &mut Heights) -> Self {
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut y = 0.0;
        tops.push(y);
        for row in rows {
            y += heights.get(row);
            tops.push(y);
        }
        Self { tops }
    }

    pub fn len(&self) -> usize {
        self.tops.len() - 1
    }

    pub fn top(&self, index: usize) -> f32 {
        self.tops[index]
    }

    pub fn height(&self, index: usize) -> f32 {
        self.tops[index + 1] - self.tops[index]
    }

    pub fn total(&self) -> f32 {
        self.tops[self.len()]
    }

    /// The row containing `y`: the first whose bottom is below it, clamped
    /// to the last row. Only meaningful for a non-empty layout.
    fn row_at(&self, y: f32) -> usize {
        self.tops[1..]
            .partition_point(|bottom| *bottom <= y)
            .min(self.len().saturating_sub(1))
    }

    /// Rows that intersect `from..to`.
    fn rows_in(&self, from: f32, to: f32) -> Range<usize> {
        if self.len() == 0 {
            return 0..0;
        }
        let start = self.row_at(from);
        let end = self.tops[..self.len()].partition_point(|top| *top < to);
        start..end.max(start + 1)
    }
}

/// What one pass draws: `above` at their estimated tops, clipped above the
/// viewport (only to measure them), then `flow` in sequence from its first
/// row's estimated top. Everything else is spacer.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub layout: Layout,
    pub above: Range<usize>,
    pub flow: Range<usize>,
}

/// One drawn row's place in this pass, from the start of the list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawn {
    pub index: usize,
    pub top: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct Anchor {
    key: String,
    /// The row's bottom below the viewport top.
    bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct Jump {
    key: String,
    since: f64,
    /// The last pass moved the offset to re-centre it.
    moved: bool,
}

/// What the list knew about the scroll area at the start of a pass.
#[derive(Debug, Clone, Copy, Default)]
pub struct Frame {
    /// The scroll offset the area is about to use.
    pub offset: f32,
    pub viewport: f32,
    /// The list is stuck to the newest row, which the area keeps in place.
    pub at_end: bool,
    /// The reader scrolled (wheel or drag) this frame.
    pub scrolled: bool,
    /// Something else sets the offset this pass; leave it alone.
    pub frozen: bool,
    pub now: f64,
}

#[derive(Default)]
pub struct VirtualList {
    heights: Heights,
    anchor: Option<Anchor>,
    jump: Option<Jump>,
    /// The conversation the anchor and jump belong to.
    scope: String,
    /// Where the list started in the scroll content last pass (below the
    /// history header), and the content after it.
    start: f32,
    tail: f32,
    rows: Vec<Row>,
    /// This pass's frame, and whether it keeps an anchor or jump in place.
    frame: Frame,
    anchoring: bool,
    /// The anchor row this pass placed, by index.
    held: Option<usize>,
}

impl VirtualList {
    /// Lays out `rows` and returns the offset this pass should use, if it
    /// must move to keep the anchor (or a jump target) in place.
    pub fn begin(
        &mut self,
        scope: &str,
        rows: Vec<Row>,
        jump: Option<String>,
        frame: Frame,
    ) -> (Layout, Option<f32>) {
        if scope != self.scope {
            self.scope = scope.to_owned();
            self.anchor = None;
            self.jump = None;
        }
        if let Some(key) = jump {
            self.jump = Some(Jump {
                key,
                since: frame.now,
                moved: false,
            });
        }
        let layout = Layout::new(&rows, &mut self.heights);
        self.rows = rows;
        self.frame = frame;
        self.anchoring = false;
        self.held = None;
        if frame.frozen {
            return (layout, None);
        }
        if frame.scrolled
            || self
                .jump
                .as_ref()
                .is_some_and(|jump| frame.now - jump.since > JUMP_SETTLE)
        {
            self.jump = None;
        }
        let offset = if let Some(jump) = &mut self.jump {
            match self.rows.iter().position(|row| row.key == jump.key) {
                Some(index) => {
                    self.anchoring = true;
                    let target = centred(&layout, index, self.start, self.tail, frame.viewport);
                    jump.moved = (target - frame.offset).abs() > EPSILON;
                    jump.moved.then_some(target)
                }
                None => {
                    self.jump = None;
                    None
                }
            }
        } else if frame.at_end {
            None
        } else {
            self.anchor.as_ref().and_then(|anchor| {
                let index = self.rows.iter().position(|row| row.key == anchor.key)?;
                self.anchoring = true;
                self.held = Some(index);
                let target = anchored(&layout, index, self.start, anchor.bottom);
                ((target - frame.offset).abs() > EPSILON).then_some(target)
            })
        };
        (layout, offset)
    }

    /// Which rows to draw with the list's viewport `height` tall at `top`
    /// (from the start of the list).
    pub fn plan(&self, layout: Layout, top: f32, height: f32) -> Plan {
        plan(layout, top, height)
    }

    /// Records this pass's measurements and the anchor for the next pass.
    /// `start` is where the list began in the scroll content, `content` the
    /// content height and `offset` the area's final offset. Returns true when
    /// this pass drew visible rows in the wrong place and should be redone.
    pub fn finish(
        &mut self,
        plan: &Plan,
        drawn: &[Drawn],
        start: f32,
        content: f32,
        offset: f32,
    ) -> bool {
        let mut changed = false;
        for row in drawn {
            if let Some(key) = self.rows.get(row.index) {
                changed |= self.heights.measure(key, row.height);
            }
        }
        self.heights.prune(&self.rows);
        self.start = start;
        self.tail = (content - start - drawn_total(plan, drawn)).max(0.0);
        let top = offset - start;
        let jumping = self.jump.as_mut().map(|jump| {
            let drawn = drawn.iter().any(|row| {
                self.rows
                    .get(row.index)
                    .is_some_and(|key| key.key == jump.key)
            });
            (drawn && !changed && !jump.moved, drawn)
        });
        if let Some((settled, _)) = jumping
            && settled
        {
            self.jump = None;
        }
        if self.frame.frozen || self.frame.at_end {
            self.anchor = self.anchor_at(plan, drawn, top);
            return false;
        }
        // Visible rows came out a different height than laid out, so the
        // anchor row (or else the first visible row) is not where this pass
        // meant to put it: keep its bottom there, and redraw.
        let held = self
            .held
            .filter(|index| plan.flow.contains(index))
            .or((!plan.flow.is_empty()).then_some(plan.flow.start));
        if self.jump.is_none()
            && let Some(index) = held
            && let Some(row) = drawn.iter().rev().find(|row| row.index == index)
        {
            let predicted = plan.layout.top(index) + plan.layout.height(index);
            if (row.top + row.height - predicted).abs() > EPSILON {
                self.anchor = Some(Anchor {
                    key: self.rows[index].key.clone(),
                    bottom: predicted - top,
                });
                return true;
            }
        }
        self.anchor = self.anchor_at(plan, drawn, top);
        jumping.is_some_and(|(_, drawn)| drawn && changed)
    }

    /// The first row in the viewport at `top`, as this pass actually drew it.
    fn anchor_at(&self, plan: &Plan, drawn: &[Drawn], top: f32) -> Option<Anchor> {
        let flow = drawn
            .iter()
            .filter(|row| plan.flow.contains(&row.index))
            .find(|row| row.top + row.height > top + EPSILON);
        if let Some(row) = flow {
            return Some(Anchor {
                key: self.rows.get(row.index)?.key.clone(),
                bottom: row.top + row.height - top,
            });
        }
        // Scrolled past everything drawn: place it from the estimates.
        if plan.layout.len() == 0 {
            return None;
        }
        let index = plan.layout.row_at(top);
        Some(Anchor {
            key: self.rows.get(index)?.key.clone(),
            bottom: plan.layout.top(index) + plan.layout.height(index) - top,
        })
    }

    /// A jump is still re-centring its target.
    pub fn jumping(&self) -> bool {
        self.jump.is_some()
    }

    /// This pass keeps an anchor (or jump target) in place, so nothing else
    /// should correct the offset for rows added above.
    pub fn anchoring(&self) -> bool {
        self.anchoring
    }
}

/// The height the flow rows actually took in place of their estimates.
fn drawn_total(plan: &Plan, drawn: &[Drawn]) -> f32 {
    let flow: f32 = drawn
        .iter()
        .filter(|row| plan.flow.contains(&row.index))
        .map(|row| row.height - plan.layout.height(row.index))
        .sum();
    plan.layout.total() + flow
}

/// The offset that puts row `index`'s bottom `bottom` below the viewport top.
fn anchored(layout: &Layout, index: usize, start: f32, bottom: f32) -> f32 {
    (start + layout.top(index) + layout.height(index) - bottom).max(0.0)
}

/// The offset that centres row `index`, within the scrollable range.
fn centred(layout: &Layout, index: usize, start: f32, tail: f32, viewport: f32) -> f32 {
    let centre = start + layout.top(index) + layout.height(index) / 2.0 - viewport / 2.0;
    let end = (start + layout.total() + tail - viewport).max(0.0);
    centre.clamp(0.0, end)
}

fn plan(layout: Layout, top: f32, height: f32) -> Plan {
    if layout.len() == 0 {
        return Plan {
            layout,
            above: 0..0,
            flow: 0..0,
        };
    }
    // An offset past the end (a jump to the latest message before it is
    // clamped) shows the last rows, not nothing.
    let height = height.max(0.0);
    let top = top.min(layout.total() - height).max(top.min(0.0));
    let bottom = top + height;
    let first = layout.row_at(top);
    let above = layout.rows_in(top - OVERSCAN, top).start.min(first)..first;
    let flow = first..layout.rows_in(top, bottom + OVERSCAN).end;
    Plan {
        layout,
        above,
        flow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The history header above the list, and the viewport height.
    const START: f32 = 40.0;
    const VIEWPORT: f32 = 500.0;

    fn rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| Row {
                key: format!("m{index}"),
                revision: 1,
            })
            .collect()
    }

    fn frame(offset: f32) -> Frame {
        Frame {
            offset,
            viewport: VIEWPORT,
            ..Default::default()
        }
    }

    /// Draws `plan` as the history does, with row `index` really
    /// `height(index)` tall.
    fn draw(plan: &Plan, height: &impl Fn(usize) -> f32) -> Vec<Drawn> {
        let mut drawn: Vec<_> = plan
            .above
            .clone()
            .map(|index| Drawn {
                index,
                top: plan.layout.top(index),
                height: height(index),
            })
            .collect();
        let mut y = if plan.flow.is_empty() {
            plan.layout.total()
        } else {
            plan.layout.top(plan.flow.start)
        };
        for index in plan.flow.clone() {
            drawn.push(Drawn {
                index,
                top: y,
                height: height(index),
            });
            y += height(index);
        }
        drawn
    }

    struct Pass {
        /// The offset the rows were drawn at.
        offset: f32,
        drawn: Vec<Drawn>,
        redo: bool,
    }

    impl Pass {
        /// Where row `index` was on screen (its top below the viewport top).
        fn screen(&self, index: usize) -> Option<f32> {
            self.drawn
                .iter()
                .find(|row| row.index == index)
                .map(|row| START + row.top - self.offset)
        }
    }

    /// One scroll-area pass: `stored` is the offset the area kept from the
    /// last pass; `wheel` moves it as the pass ends, like egui does.
    fn pass(
        list: &mut VirtualList,
        rows: Vec<Row>,
        stored: f32,
        wheel: f32,
        jump: Option<&str>,
        height: impl Fn(usize) -> f32,
    ) -> (Pass, f32) {
        let (layout, moved) = list.begin("c", rows, jump.map(str::to_owned), frame(stored));
        let offset = moved.unwrap_or(stored);
        let plan = list.plan(layout, offset - START, VIEWPORT);
        let drawn = draw(&plan, &height);
        let content = START + drawn_total(&plan, &drawn);
        let end = (offset + wheel).clamp(0.0, (content - VIEWPORT).max(0.0));
        let redo = list.finish(&plan, &drawn, START, content, end);
        (
            Pass {
                offset,
                drawn,
                redo,
            },
            end,
        )
    }

    /// Alternating tall and short rows, unlike the default estimate.
    fn uneven(index: usize) -> f32 {
        if index.is_multiple_of(2) { 90.0 } else { 30.0 }
    }

    #[test]
    fn layout_finds_rows_and_ranges_by_position() {
        let mut heights = Heights::default();
        let rows = rows(4);
        for (row, height) in rows.iter().zip([10.0, 20.0, 30.0, 40.0]) {
            heights.measure(row, height);
        }
        let layout = Layout::new(&rows, &mut heights);
        assert_eq!(layout.total(), 100.0);
        assert_eq!((layout.top(2), layout.height(2)), (30.0, 30.0));
        assert_eq!(layout.row_at(-5.0), 0);
        assert_eq!(layout.row_at(10.0), 1);
        assert_eq!(layout.row_at(29.9), 1);
        assert_eq!(layout.row_at(500.0), 3);
        assert_eq!(layout.rows_in(15.0, 31.0), 1..3);
        assert_eq!(layout.rows_in(30.0, 60.0), 2..3);
        assert_eq!(layout.rows_in(0.0, 0.0), 0..1);
    }

    #[test]
    fn unmeasured_rows_keep_their_first_estimate() {
        let mut heights = Heights::default();
        let rows = rows(3);
        assert_eq!(heights.get(&rows[2]), DEFAULT_HEIGHT);
        assert!(heights.measure(&rows[0], 100.0));
        // The mean moved, but row 2 was already laid out with the old one.
        assert_eq!(heights.estimate(), 100.0);
        assert_eq!(heights.get(&rows[2]), DEFAULT_HEIGHT);
        assert_eq!(heights.get(&rows[1]), 100.0);
        assert!(
            !heights.measure(&rows[0], 100.2),
            "rounding is not a change"
        );
        assert!(heights.measure(&rows[2], 70.0));
        assert_eq!(heights.estimate(), 85.0);
    }

    #[test]
    fn stale_revisions_keep_their_last_height_until_redrawn() {
        let mut heights = Heights::default();
        let mut row = rows(1).remove(0);
        heights.measure(&row, 80.0);
        row.revision = 2;
        assert_eq!(heights.get(&row), 80.0);
        assert!(!heights.measure(&row, 80.0));
        assert!(heights.measure(&row, 120.0));
        assert_eq!(heights.estimate(), 120.0);
    }

    #[test]
    fn many_conversations_do_not_grow_the_cache_without_bound() {
        let mut heights = Heights::default();
        let many: Vec<Row> = (0..RETAINED + 100)
            .map(|index| Row {
                key: format!("old{index}"),
                revision: 1,
            })
            .collect();
        for row in &many {
            heights.measure(row, 10.0);
        }
        let current = rows(50);
        for row in &current {
            heights.measure(row, 30.0);
        }
        heights.prune(&current);
        assert_eq!(heights.rows.len(), 50);
        assert_eq!(heights.estimate(), 30.0);
    }

    #[test]
    fn plans_only_rows_near_the_viewport() {
        let mut heights = Heights::default();
        let rows = rows(1_000);
        for row in &rows {
            heights.measure(row, 50.0);
        }
        let layout = Layout::new(&rows, &mut heights);
        let middle = plan(layout.clone(), 10_000.0, 500.0);
        // OVERSCAN beyond each edge of the viewport.
        assert_eq!(middle.above, 188..200);
        assert_eq!(middle.flow, 200..222);
        // An offset past the end (clamped only as the pass ends) draws the
        // last rows, not nothing.
        let end = plan(layout.clone(), f32::MAX, 500.0);
        assert_eq!((end.above, end.flow), (978..990, 990..1_000));
        let start = plan(layout, -START, VIEWPORT);
        assert_eq!((start.above, start.flow), (0..0, 0..22));
        let empty = plan(Layout { tops: vec![0.0] }, 0.0, VIEWPORT);
        assert_eq!((empty.above, empty.flow), (0..0, 0..0));
    }

    #[test]
    fn rows_measured_above_the_reader_never_move_what_they_read() {
        let mut list = VirtualList::default();
        let mut stored = 0.0;
        for _ in 0..3 {
            stored = pass(&mut list, rows(300), stored, 0.0, None, uneven).1;
        }
        // Drag far into rows that were never measured, then let the overscan
        // measure the rows around the viewport.
        stored = pass(&mut list, rows(300), stored, 9_000.0, None, uneven).1;
        assert!(stored > 8_000.0);
        let mut last = None;
        for _ in 0..4 {
            let (drawn, end) = pass(&mut list, rows(300), stored, 0.0, None, uneven);
            stored = end;
            last = Some(drawn);
        }
        let last = last.unwrap();
        let watched = last
            .drawn
            .iter()
            .find(|row| START + row.top - last.offset > 0.0)
            .unwrap()
            .index;
        let mut at = last.screen(watched).unwrap();
        // Wheel up 100 points a pass: each pass measures rows entering the
        // overscan above, yet the content moves by exactly the wheel.
        let mut checked = 0;
        for _ in 0..30 {
            let (drawn, end) = pass(&mut list, rows(300), stored, -100.0, None, uneven);
            assert!(!drawn.redo);
            stored = end;
            let Some(now) = drawn.screen(watched) else {
                break;
            };
            if checked > 0 {
                assert!((now - at - 100.0).abs() < 0.5, "moved to {now} from {at}");
            }
            at = now;
            checked += 1;
        }
        assert!(
            checked >= 5,
            "watched row stayed on screen for {checked} passes"
        );
    }

    #[test]
    fn older_history_prepended_keeps_the_reading_position() {
        let mut list = VirtualList::default();
        let mut stored = 0.0;
        for _ in 0..3 {
            stored = pass(&mut list, rows(40), stored, 0.0, None, |_| 45.0).1;
        }
        assert_eq!(stored, 0.0);
        // A page arrives above while reading the top: "m0" is now row 50.
        let joined: Vec<Row> = (0..50)
            .map(|index| Row {
                key: format!("old{index}"),
                revision: 1,
            })
            .chain(rows(40))
            .collect();
        let older = |index: usize| if index < 50 { 70.0 } else { 45.0 };
        let (arrived, mut stored) = pass(&mut list, joined.clone(), stored, 0.0, None, older);
        assert!(arrived.offset > 2_000.0);
        // The row above it, partly in view, was taller than estimated: the
        // pass is redone (in the same frame) with that row grown upwards.
        assert!(arrived.redo);
        let (redone, end) = pass(&mut list, joined.clone(), stored, 0.0, None, older);
        assert!(!redone.redo);
        assert_eq!(redone.screen(50), Some(START));
        // Nothing moves as later passes measure the new rows.
        stored = end;
        for _ in 0..3 {
            let (settled, end) = pass(&mut list, joined.clone(), stored, 0.0, None, older);
            assert_eq!(settled.screen(50), Some(START));
            stored = end;
        }
    }

    #[test]
    fn an_anchor_row_that_changes_height_keeps_its_bottom_and_redoes_the_pass() {
        let mut list = VirtualList::default();
        let mut stored = 1_000.0;
        let mut last = None;
        for _ in 0..4 {
            let (drawn, end) = pass(&mut list, rows(100), stored, 0.0, None, |_| 50.0);
            stored = end;
            last = Some(drawn);
        }
        let last = last.unwrap();
        let first = last
            .drawn
            .iter()
            .find(|row| START + row.top + row.height - last.offset > 0.5)
            .unwrap()
            .index;
        let bottom = last.screen(first).unwrap() + 50.0;
        // The first visible row loses a 20-point header (a revision change):
        // the text below it stays put, after one redone pass.
        let mut changed = rows(100);
        changed[first].revision = 2;
        let shorter = |index: usize| if index == first { 30.0 } else { 50.0 };
        let (redone, end) = pass(&mut list, changed.clone(), stored, 0.0, None, shorter);
        assert!(redone.redo, "the visible row moved, so the pass is redone");
        let (fixed, _) = pass(&mut list, changed, end, 0.0, None, shorter);
        assert!(!fixed.redo);
        assert_eq!(fixed.screen(first).map(|top| top + 30.0), Some(bottom));
        assert_eq!(fixed.screen(first + 1), Some(bottom));
    }

    #[test]
    fn the_live_end_is_left_to_stick_to_bottom() {
        let mut list = VirtualList::default();
        for _ in 0..3 {
            pass(&mut list, rows(50), 1_000.0, 0.0, None, |_| 50.0);
        }
        let mut grown = rows(50);
        grown.insert(
            0,
            Row {
                key: "older".into(),
                revision: 1,
            },
        );
        let at_end = Frame {
            at_end: true,
            ..frame(1_000.0)
        };
        assert_eq!(list.begin("c", grown, None, at_end).1, None);
        assert!(!list.anchoring());
    }

    #[test]
    fn jumps_centre_their_target_until_it_settles_or_the_reader_scrolls() {
        let mut list = VirtualList::default();
        let height = |index: usize| 40.0 + (index % 3) as f32 * 25.0;
        let mut stored = 0.0;
        for _ in 0..2 {
            stored = pass(&mut list, rows(300), stored, 0.0, None, height).1;
        }
        let (jumped, end) = pass(&mut list, rows(300), stored, 0.0, Some("m150"), height);
        assert!(jumped.screen(150).is_some(), "the target is drawn at once");
        assert!(list.anchoring());
        // Passes re-centre as its neighbours are measured, then settle.
        let mut stored = end;
        for _ in 0..6 {
            if !list.jumping() {
                break;
            }
            stored = pass(&mut list, rows(300), stored, 0.0, None, height).1;
        }
        assert!(!list.jumping());
        let (settled, stored) = pass(&mut list, rows(300), stored, 0.0, None, height);
        let centre = settled.screen(150).unwrap() + height(150) / 2.0;
        assert!((centre - VIEWPORT / 2.0).abs() < 0.5, "centred at {centre}");
        // A jump the reader scrolls away from stops re-centring.
        list.begin("c", rows(300), Some("m10".into()), frame(stored));
        assert!(list.jumping());
        let scrolled = Frame {
            scrolled: true,
            ..frame(stored)
        };
        assert_eq!(list.begin("c", rows(300), None, scrolled).1, None);
        assert!(!list.jumping());
        // So does one that cannot settle, after a moment.
        list.begin("c", rows(300), Some("m10".into()), frame(stored));
        let late = Frame {
            now: JUMP_SETTLE + 0.5,
            ..frame(stored)
        };
        list.begin("c", rows(300), None, late);
        assert!(!list.jumping());
    }

    #[test]
    fn another_conversation_drops_the_anchor() {
        let mut list = VirtualList::default();
        for _ in 0..3 {
            pass(&mut list, rows(100), 2_000.0, 0.0, None, |_| 50.0);
        }
        assert!(list.anchor.is_some());
        let (_, moved) = list.begin("other", rows(100), None, frame(0.0));
        assert_eq!(moved, None);
        assert!(list.anchor.is_none());
    }
}
