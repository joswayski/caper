//! View reactions follows the message's live, projected reaction IDs. Cached
//! identities survive refreshes; an unresolved failed load waits for Retry.
use crate::avatars::{self, Avatar};
use crate::gfx::{Gfx, fill_round, rect, stroke_round};
use crate::overlay::{Dialog, Kind, Part, Rect, height, layout, width};
use crate::{App, EV_FOCUS, EV_SCREEN, theme::*};
use caper_client::model::{self, Reaction, Reactor};
use caper_client::reactions::{ReactorCache, ReactorState};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::core::Result;

#[derive(PartialEq)]
struct Shown {
    tabs: Vec<(String, usize)>,
    selected: String,
    people: std::result::Result<Vec<Reactor>, bool>,
}

impl Shown {
    fn new(
        reactions: &[Reaction],
        selected: &str,
        cached: Option<&ReactorCache>,
        me: &Reactor,
    ) -> Option<Self> {
        let current = reactions
            .iter()
            .find(|r| r.emoji == selected && !r.author_ids.is_empty())
            .or_else(|| reactions.iter().find(|r| !r.author_ids.is_empty()))?;
        let failed = cached.is_some_and(|c| matches!(c.state, ReactorState::Failed(_)));
        let people = model::reactor_list(
            cached.map_or(&[], |c| c.groups.as_slice()),
            current,
            Some(&me.id),
        )
        .map(|people| {
            people
                .into_iter()
                .map(|person| {
                    if person.id == me.id
                        && person.username.is_none()
                        && person.display_name.is_none()
                    {
                        me.clone()
                    } else {
                        person
                    }
                })
                .collect()
        })
        .ok_or(failed);
        Some(Self {
            tabs: reactions
                .iter()
                .filter(|r| !r.author_ids.is_empty())
                .map(|r| (r.emoji.clone(), r.author_ids.len()))
                .collect(),
            selected: current.emoji.clone(),
            people,
        })
    }
}

pub struct View {
    pub id: String,
    pub focus: Part,
    shown: Shown,
}

fn button(p: Part) -> bool {
    matches!(p, Part::Close | Part::Retry | Part::ReactorTab(_))
}

impl Dialog {
    pub fn reactor_name(&self, p: Part) -> String {
        let Some(v) = &self.reactors else {
            return String::new();
        };
        match p {
            Part::Title => "Reactions".into(),
            Part::Close => "Close reactions".into(),
            Part::Retry => "Retry".into(),
            Part::Loading => "Loading reactions…".into(),
            Part::Error => "Couldn’t load reactions.".into(),
            Part::Label => caper_client::emoji::name(&v.shown.selected)
                .map_or_else(|| v.shown.selected.clone(), |n| format!(":{n}:")),
            Part::ReactorTab(k) => {
                v.shown
                    .tabs
                    .get(k)
                    .map_or_else(String::new, |(emoji, count)| {
                        format!(
                            "{emoji}, {count}{}",
                            if *emoji == v.shown.selected {
                                ", selected"
                            } else {
                                ""
                            }
                        )
                    })
            }
            Part::ReactorPerson(k) => v
                .shown
                .people
                .as_ref()
                .ok()
                .and_then(|p| p.get(k))
                .map_or_else(String::new, |p| {
                    let name = model::reactor_name(p);
                    p.username
                        .as_ref()
                        .map_or_else(|| name.clone(), |u| format!("{name}, @{u}"))
                }),
            _ => String::new(),
        }
    }

    pub fn reactor_bounds(&self, p: Part) -> Option<Rect> {
        let (x, y, w, h) = self.part_rect(p)?;
        if !p.in_body() {
            return Some((x, y, w, h));
        }
        let top = y.max(self.body.1);
        Some((
            x,
            top,
            w,
            ((y + h).min(self.body.1 + self.body.3) - top).max(0.0),
        ))
    }

    pub fn reactor_focus(&mut self, p: Part) {
        if !button(p) || self.part_rect(p).is_none() {
            return;
        }
        if let Some(v) = &mut self.reactors {
            v.focus = p;
        }
        if p.in_body()
            && let Some((_, y, _, h)) = self.part_rect(p)
        {
            if y < self.body.1 {
                self.scroll -= self.body.1 - y;
            } else if y + h > self.body.1 + self.body.3 {
                self.scroll += y + h - self.body.1 - self.body.3;
            }
        }
    }

    pub fn relayout_reactors(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let cw = (w - 32.0).clamp(200.0, 368.0);
        let inner = cw - 32.0;
        let v = self.reactors.as_ref().unwrap();
        let mut parts = Vec::new();
        let (mut x, mut y) = (0.0, 0.0);
        for (k, (emoji, count)) in v.shown.tabs.iter().enumerate() {
            let tw = (width(&layout(
                g,
                &format!("{emoji}  {count}"),
                &g.formats.button,
                inner,
            )?) + 20.0)
                .min(inner);
            if x > 0.0 && x + tw > inner {
                x = 0.0;
                y += 38.0;
            }
            parts.push((Part::ReactorTab(k), (x, y, tw, 32.0)));
            x += tw + 4.0;
        }
        y += 44.0;
        parts.push((Part::Label, (0.0, y, inner, 22.0)));
        y += 28.0;
        match &v.shown.people {
            Ok(people) => {
                for (k, person) in people.iter().enumerate() {
                    let name = layout(
                        g,
                        &model::reactor_name(person),
                        &g.formats.name,
                        inner - 42.0,
                    )?;
                    let user = person
                        .username
                        .as_ref()
                        .map(|u| layout(g, &format!("@{u}"), &g.formats.time, inner - 42.0))
                        .transpose()?;
                    let rh = (height(&name) + user.as_ref().map_or(0.0, height)).max(32.0) + 10.0;
                    parts.push((Part::ReactorPerson(k), (0.0, y, inner, rh)));
                    y += rh;
                }
            }
            Err(failed) => {
                parts.push((
                    if *failed { Part::Error } else { Part::Loading },
                    (0.0, y, inner, 28.0),
                ));
                y += 34.0;
                if *failed {
                    parts.push((Part::Retry, (0.0, y, inner, 32.0)));
                    y += 40.0;
                }
            }
        }
        let bh = y.min(390.0).min((h - 100.0).max(40.0));
        let ch = bh + 76.0;
        let (cx, cy) = ((w - cw) / 2.0, (h - ch) / 2.0);
        self.card = (cx, cy, cw, ch);
        self.body = (cx + 16.0, cy + 58.0, inner, bh);
        self.scroll = self.scroll.clamp(0.0, (y - bh).max(0.0));
        for (_, r) in &mut parts {
            r.0 += self.body.0;
            r.1 += self.body.1 - self.scroll;
        }
        parts.insert(0, (Part::Title, (cx + 16.0, cy + 16.0, inner - 40.0, 28.0)));
        parts.push((Part::Close, (cx + cw - 44.0, cy + 12.0, 32.0, 32.0)));
        self.parts = parts;
        Ok(())
    }

    pub fn draw_reactors(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let v = self.reactors.as_ref().unwrap();
        for &(p, (x, y, w, h)) in &self.parts {
            unsafe {
                if p.in_body() {
                    dc.PushAxisAlignedClip(
                        &rect(
                            self.body.0,
                            self.body.1,
                            self.body.0 + self.body.2,
                            self.body.1 + self.body.3,
                        ),
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                }
                let r = rect(x, y, x + w, y + h);
                if let Part::ReactorPerson(k) = p {
                    let person = &v.shown.people.as_ref().unwrap()[k];
                    avatars::draw(
                        dc,
                        &Avatar::new(person.avatar_id, &person.avatar_asset, &person.avatar_url),
                        rect(x, y + 3.0, x + 32.0, y + 35.0),
                        RADIUS,
                    );
                    let name = layout(g, &model::reactor_name(person), &g.formats.name, w - 42.0)?;
                    crate::text::draw(dc, &name, x + 42.0, y + 3.0, &g.brushes.text);
                    if let Some(username) = &person.username {
                        let user = layout(g, &format!("@{username}"), &g.formats.time, w - 42.0)?;
                        crate::text::draw(
                            dc,
                            &user,
                            x + 42.0,
                            y + 3.0 + height(&name),
                            &g.brushes.muted,
                        );
                    }
                } else {
                    let selected =
                        matches!(p, Part::ReactorTab(k) if v.shown.tabs[k].0 == v.shown.selected);
                    if button(p) {
                        fill_round(
                            dc,
                            r,
                            RADIUS,
                            if self.hover == Some(p) {
                                &g.brushes.hover
                            } else {
                                &g.brushes.surface
                            },
                        );
                        stroke_round(
                            dc,
                            r,
                            RADIUS,
                            if selected || v.focus == p {
                                &g.brushes.terracotta
                            } else {
                                &g.brushes.border
                            },
                        );
                    }
                    let label = match p {
                        Part::Close => "×".into(),
                        Part::ReactorTab(k) => {
                            format!("{}  {}", v.shown.tabs[k].0, v.shown.tabs[k].1)
                        }
                        _ => self.reactor_name(p),
                    };
                    let format = if p == Part::Title {
                        &g.formats.dialog_title
                    } else if button(p) {
                        &g.formats.button
                    } else {
                        &g.formats.dialog_note
                    };
                    let l = layout(g, &label, format, if button(p) { w - 16.0 } else { w })?;
                    crate::text::draw(
                        dc,
                        &l,
                        x + if button(p) { 8.0 } else { 0.0 },
                        y + if button(p) {
                            (h - height(&l)) / 2.0
                        } else {
                            0.0
                        },
                        if p == Part::Error {
                            &g.brushes.error
                        } else {
                            &g.brushes.text
                        },
                    );
                }
                if p.in_body() {
                    dc.PopAxisAlignedClip();
                }
            }
        }
        Ok(())
    }
}

impl App {
    pub fn open_reactors(&mut self, id: &str) {
        let Some(live) = &mut self.live else { return };
        let Some(message) = live.reactor_message(id) else {
            return;
        };
        let Some(shown) = Shown::new(
            &message.reactions,
            "",
            live.reactor_cache(id),
            &live.reactor_self(),
        ) else {
            return;
        };
        let mut d = Dialog::new(Kind::Reactions, Some(message), Default::default());
        d.reactors = Some(Box::new(View {
            id: id.to_owned(),
            focus: Part::Close,
            shown,
        }));
        self.open_dialog(d);
        self.refresh_reactors();
    }

    pub fn refresh_reactors(&mut self) {
        let Some(view) = self.dialog.as_ref().and_then(|d| d.reactors.as_ref()) else {
            return;
        };
        let Some(live) = &mut self.live else {
            self.close_dialog();
            return;
        };
        let Some(message) = live.reactor_message(&view.id) else {
            self.close_dialog();
            return;
        };
        let revision = message.reaction_seq.as_deref().unwrap_or("0");
        if !live
            .reactor_cache(&view.id)
            .is_some_and(|c| c.revision == revision && matches!(c.state, ReactorState::Failed(_)))
        {
            live.load_reactors(&view.id);
        }
        let Some(shown) = Shown::new(
            &message.reactions,
            &view.shown.selected,
            live.reactor_cache(&view.id),
            &live.reactor_self(),
        ) else {
            self.close_dialog();
            return;
        };
        if shown == view.shown {
            return;
        }
        let d = self.dialog.as_mut().unwrap();
        let view = d.reactors.as_mut().unwrap();
        if shown.selected != view.shown.selected {
            d.scroll = 0.0;
            view.focus = Part::Close;
        } else if let Part::ReactorTab(k) = view.focus {
            view.focus = view
                .shown
                .tabs
                .get(k)
                .and_then(|old| shown.tabs.iter().position(|new| new.0 == old.0))
                .map_or(Part::Close, Part::ReactorTab);
        }
        view.shown = shown;
        self.dialog_changed();
        if let Some(d) = &mut self.dialog {
            let focus = d.reactors.as_ref().unwrap().focus;
            if d.part_rect(focus).is_none() {
                d.reactors.as_mut().unwrap().focus = Part::Close;
            }
        }
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    pub fn reactors_activate(&mut self, p: Part) {
        let Some(d) = &mut self.dialog else { return };
        d.reactor_focus(p);
        let Some(v) = &mut d.reactors else { return };
        match p {
            Part::Close => {
                self.close_dialog();
                return;
            }
            Part::Retry => {
                if let Some(live) = &mut self.live {
                    live.retry_reactors(&v.id);
                }
            }
            Part::ReactorTab(k) => {
                if let Some((emoji, _)) = v.shown.tabs.get(k) {
                    v.shown.selected = emoji.clone();
                    d.scroll = 0.0;
                }
            }
            _ => return,
        }
        self.refresh_reactors();
        self.dialog_changed();
        self.events |= EV_FOCUS;
    }

    pub fn reactors_key(&mut self, vk: VIRTUAL_KEY, shift: bool) {
        let Some(d) = &mut self.dialog else { return };
        let Some(v) = &d.reactors else { return };
        let focus = v.focus;
        match vk {
            VK_ESCAPE => self.close_dialog(),
            VK_RETURN | VK_SPACE => self.reactors_activate(focus),
            VK_TAB => {
                let choices: Vec<_> = d.parts().filter(|p| button(*p)).collect();
                let i = choices.iter().position(|p| *p == focus).unwrap_or(0);
                d.reactor_focus(
                    choices[(i + if shift { choices.len() - 1 } else { 1 }) % choices.len()],
                );
            }
            VK_LEFT | VK_RIGHT => {
                let n = v.shown.tabs.len();
                let k = v
                    .shown
                    .tabs
                    .iter()
                    .position(|t| t.0 == v.shown.selected)
                    .unwrap_or(0);
                self.reactors_activate(Part::ReactorTab(
                    (k + if vk == VK_LEFT { n - 1 } else { 1 }) % n,
                ));
            }
            VK_UP => d.scroll -= 42.0,
            VK_DOWN => d.scroll += 42.0,
            VK_PRIOR => d.scroll -= d.body.3,
            VK_NEXT => d.scroll += d.body.3,
            VK_HOME => d.scroll = 0.0,
            VK_END => d.scroll = f32::MAX / 2.0,
            _ => return,
        }
        self.dialog_changed();
        self.events |= EV_FOCUS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::model::ReactorGroup;

    #[test]
    fn live_snapshot_keeps_names_moves_removed_selection_and_distinguishes_retry() {
        let me = Reactor {
            id: "me".into(),
            username: Some("alice".into()),
            display_name: Some("Alice".into()),
            avatar_id: Some(9),
            avatar_asset: None,
            avatar_url: None,
        };
        let bob = Reactor {
            id: "bob".into(),
            username: Some("bob".into()),
            display_name: Some("Bob".into()),
            ..me.clone()
        };
        let thumbs = Reaction {
            emoji: "👍".into(),
            author_ids: vec!["bob".into(), "me".into()],
        };
        let party = Reaction {
            emoji: "🎉".into(),
            author_ids: vec!["bob".into()],
        };
        let mut cache = ReactorCache {
            revision: "4".into(),
            state: ReactorState::Failed(std::time::Instant::now()),
            groups: Vec::new(),
        };
        assert_eq!(
            Shown::new(std::slice::from_ref(&thumbs), "👍", None, &me)
                .unwrap()
                .people,
            Err(false)
        );
        assert_eq!(
            Shown::new(std::slice::from_ref(&thumbs), "👍", Some(&cache), &me)
                .unwrap()
                .people,
            Err(true)
        );
        cache.groups.push(ReactorGroup {
            emoji: "👍".into(),
            authors: vec![bob.clone()],
        });
        let view = Shown::new(&[thumbs.clone(), party], "🎉", Some(&cache), &me).unwrap();
        assert_eq!(view.selected, "🎉");
        assert_eq!(view.people.unwrap(), std::slice::from_ref(&bob));
        let view = Shown::new(&[thumbs], &view.selected, Some(&cache), &me).unwrap();
        assert_eq!(view.selected, "👍");
        assert_eq!(view.tabs, [("👍".into(), 2)]);
        assert_eq!(view.people.unwrap(), [bob, me.clone()]);
        let own = Reaction {
            emoji: "👍".into(),
            author_ids: vec!["me".into()],
        };
        assert_eq!(
            Shown::new(&[own], "👍", Some(&cache), &me)
                .unwrap()
                .people
                .unwrap(),
            std::slice::from_ref(&me)
        );
        assert!(Shown::new(&[], "👍", Some(&cache), &me).is_none());
    }
}
