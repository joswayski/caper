//! Which characters an edit-history comparison highlights, as web's history
//! view shows them (`apps/web/src/chat/MessageDiff.tsx`: @pierre/diffs with
//! `lineDiffType: "word"` over jsdiff 9.0.0). Lines are diffed first; each
//! changed line paired with its counterpart is then diffed by words
//! (`diffWordsWithSpace`), and a changed line without a counterpart is
//! highlighted whole. Shared by the native clients (`apps/native/client`, and
//! Android and Apple through `apps/native/mobile`); every client runs
//! `shared/messages/history-diff-cases.json`, generated from web.

use std::ops::Range;

/// Byte ranges covering the whole text, in order, each marked when it is
/// highlighted. Adjacent ranges never share a mark.
pub type Highlights = Vec<(Range<usize>, bool)>;

/// The before and after texts' highlights.
pub fn highlights(before: &str, after: &str) -> (Highlights, Highlights) {
    let mut old = vec![false; before.len()];
    let mut new = vec![false; after.len()];
    if before != after {
        mark_lines(before, after, &mut old, &mut new);
    }
    (runs(&old), runs(&new))
}

fn runs(flags: &[bool]) -> Highlights {
    let mut out: Highlights = Vec::new();
    for (index, &flag) in flags.iter().enumerate() {
        match out.last_mut() {
            Some((range, last)) if *last == flag => range.end = index + 1,
            _ => out.push((index..index + 1, flag)),
        }
    }
    out
}

/// jsdiff's `\s`: Unicode white space plus U+FEFF, without U+0085.
fn js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// Lines keep their `\n` (jsdiff's line tokens and @pierre/diffs' file lines).
fn lines(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (index, _) in text.match_indices('\n') {
        out.push(start..index + 1);
        start = index + 1;
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

fn mark_lines(before: &str, after: &str, old: &mut [bool], new: &mut [bool]) {
    let old_lines = lines(before);
    let new_lines = lines(after);
    let old_tokens: Vec<&str> = old_lines.iter().map(|r| &before[r.clone()]).collect();
    let new_tokens: Vec<&str> = new_lines.iter().map(|r| &after[r.clone()]).collect();
    // Change blocks: the runs between unchanged lines.
    let mut blocks: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let (mut o, mut n) = (0, 0);
    let mut open: Option<(usize, usize)> = None;
    for (count, change) in myers(&old_tokens, &new_tokens) {
        match change {
            Change::Same => {
                if let Some((a, b)) = open.take() {
                    blocks.push((a..o, b..n));
                }
                o += count;
                n += count;
            }
            Change::Removed | Change::Added => {
                open.get_or_insert((o, n));
                if change == Change::Removed {
                    o += count;
                } else {
                    n += count;
                }
            }
        }
    }
    if let Some((a, b)) = open {
        blocks.push((a..o, b..n));
    }
    for (deleted, added) in blocks {
        for (d, a) in realign(&old_tokens, &new_tokens, deleted, added) {
            match (d, a) {
                (Some(d), Some(a)) => mark_words(
                    before,
                    old_lines[d].clone(),
                    after,
                    new_lines[a].clone(),
                    old,
                    new,
                ),
                (Some(d), None) => old[old_lines[d].clone()].fill(true),
                (None, Some(a)) => new[new_lines[a].clone()].fill(true),
                (None, None) => {}
            }
        }
    }
}

/// @pierre/diffs' `realignChangeContentBySimilarity`: a block with more lines
/// on one side pairs its lines at the offset whose contents look most alike.
fn realign(
    old: &[&str],
    new: &[&str],
    deleted: Range<usize>,
    added: Range<usize>,
) -> Vec<(Option<usize>, Option<usize>)> {
    let (deletions, additions) = (deleted.len(), added.len());
    let pairs = deletions.min(additions);
    let surplus = deletions.abs_diff(additions);
    let mut offset = 0;
    if pairs > 0 && surplus > 0 && pairs * (surplus + 1) <= 4096 {
        let strip = |line: &str| -> Vec<u16> {
            line.chars()
                .filter(|c| !js_space(*c))
                .collect::<String>()
                .encode_utf16()
                .collect()
        };
        let old: Vec<_> = deleted.clone().map(|i| strip(old[i])).collect();
        let new: Vec<_> = added.clone().map(|i| strip(new[i])).collect();
        let longer = additions > deletions;
        let mut best = -1.0;
        for candidate in 0..=surplus {
            let score: f64 = (0..pairs)
                .map(|pair| {
                    let (d, a) = if longer {
                        (pair, pair + candidate)
                    } else {
                        (pair + candidate, pair)
                    };
                    similarity(&old[d], &new[a])
                })
                .sum();
            if candidate == 0 {
                best = score + pairs as f64 * 0.5;
            } else if score > best {
                best = score;
                offset = candidate;
            }
        }
    }
    let (skip_old, skip_new) = if additions > deletions {
        (0, offset)
    } else {
        (offset, 0)
    };
    let mut out = Vec::new();
    out.extend((0..skip_old).map(|i| (Some(deleted.start + i), None)));
    out.extend((0..skip_new).map(|i| (None, Some(added.start + i))));
    if offset == 0 {
        for i in 0..deletions.max(additions) {
            out.push((
                (i < deletions).then_some(deleted.start + i),
                (i < additions).then_some(added.start + i),
            ));
        }
    } else {
        out.extend((0..pairs).map(|i| {
            (
                Some(deleted.start + skip_old + i),
                Some(added.start + skip_new + i),
            )
        }));
        out.extend((skip_old + pairs..deletions).map(|i| (Some(deleted.start + i), None)));
        out.extend((skip_new + pairs..additions).map(|i| (None, Some(added.start + i))));
    }
    out
}

fn similarity(a: &[u16], b: &[u16]) -> f64 {
    if a == b {
        return 1.0;
    }
    let (longest, shortest) = (a.len().max(b.len()), a.len().min(b.len()));
    if shortest == 0 {
        return 0.0;
    }
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take(shortest - prefix)
        .take_while(|(x, y)| x == y)
        .count();
    (prefix + suffix) as f64 / longest as f64
}

/// Without the line's final `\n` or `\r\n`, as @pierre/diffs compares it.
fn content(text: &str, line: Range<usize>) -> Range<usize> {
    let mut end = line.end;
    if text[line.start..end].ends_with('\n') {
        end -= 1;
        if text[line.start..end].ends_with('\r') {
            end -= 1;
        }
    }
    line.start..end
}

fn mark_words(
    before: &str,
    old_line: Range<usize>,
    after: &str,
    new_line: Range<usize>,
    old: &mut [bool],
    new: &mut [bool],
) {
    let (old_line, new_line) = (content(before, old_line), content(after, new_line));
    let utf16 = |text: &str| text.encode_utf16().count();
    if utf16(&before[old_line.clone()]) > 8000 || utf16(&after[new_line.clone()]) > 8000 {
        return;
    }
    let old_words = words(&before[old_line.clone()]);
    let new_words = words(&after[new_line.clone()]);
    let old_tokens: Vec<&str> = old_words
        .iter()
        .map(|r| &before[old_line.start..][r.clone()])
        .collect();
    let new_tokens: Vec<&str> = new_words
        .iter()
        .map(|r| &after[new_line.start..][r.clone()])
        .collect();
    let (mut o, mut n) = (0, 0);
    for (count, change) in myers(&old_tokens, &new_tokens) {
        let span = |words: &[Range<usize>], at: usize, start: usize| {
            start + words[at].start..start + words[at + count - 1].end
        };
        match change {
            Change::Same => {
                o += count;
                n += count;
            }
            Change::Removed => {
                old[span(&old_words, o, old_line.start)].fill(true);
                o += count;
            }
            Change::Added => {
                new[span(&new_words, n, new_line.start)].fill(true);
                n += count;
            }
        }
    }
}

/// `diffWordsWithSpace` tokens: a newline, a run of word characters, a run of
/// other white space, or any other single character.
fn words(text: &str) -> Vec<Range<usize>> {
    // jsdiff's extended word characters: ASCII letters, digits, `_` and Latin letters.
    let word = |c: char| {
        matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '\u{AD}' | '\u{C0}'..='\u{D6}'
            | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2C6}' | '\u{2C8}'..='\u{2D7}'
            | '\u{2DE}'..='\u{2FF}' | '\u{1E00}'..='\u{1EFF}')
    };
    let space = |c: char| js_space(c) && c != '\n' && c != '\r';
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let mut end = start + c.len_utf8();
        let run = |chars: &mut std::iter::Peekable<std::str::CharIndices>,
                   end: &mut usize,
                   keep: &dyn Fn(char) -> bool| {
            while let Some(&(at, next)) = chars.peek() {
                if !keep(next) {
                    break;
                }
                *end = at + next.len_utf8();
                chars.next();
            }
        };
        if c == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            chars.next();
            end += 1;
        } else if word(c) {
            run(&mut chars, &mut end, &word);
        } else if space(c) {
            run(&mut chars, &mut end, &space);
        }
        out.push(start..end);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Change {
    Same,
    Removed,
    Added,
}

#[derive(Clone, Copy)]
struct Component {
    count: usize,
    change: Change,
    previous: Option<usize>,
}

#[derive(Clone, Copy)]
struct Path {
    old: isize,
    last: Option<usize>,
}

/// jsdiff's `Diff.diffWithOptionsObj` (Myers with its pruning and tie
/// rules), returning runs of tokens in order.
fn myers(old: &[&str], new: &[&str]) -> Vec<(usize, Change)> {
    let (old_len, new_len) = (old.len() as isize, new.len() as isize);
    let mut components: Vec<Component> = Vec::new();
    let extract = |components: &mut Vec<Component>, path: &mut Path, diagonal: isize| -> isize {
        let (mut o, mut n) = (path.old, path.old - diagonal);
        let mut common = 0;
        while n + 1 < new_len && o + 1 < old_len && old[(o + 1) as usize] == new[(n + 1) as usize] {
            n += 1;
            o += 1;
            common += 1;
        }
        if common > 0 {
            components.push(Component {
                count: common,
                change: Change::Same,
                previous: path.last,
            });
            path.last = Some(components.len() - 1);
        }
        path.old = o;
        n
    };
    let add_to_path =
        |components: &mut Vec<Component>, path: Path, change: Change, step: isize| -> Path {
            let component = match path.last.map(|last| components[last]) {
                Some(last) if last.change == change => Component {
                    count: last.count + 1,
                    change,
                    previous: last.previous,
                },
                _ => Component {
                    count: 1,
                    change,
                    previous: path.last,
                },
            };
            components.push(component);
            Path {
                old: path.old + step,
                last: Some(components.len() - 1),
            }
        };
    let max_edit = old_len + new_len;
    let offset = max_edit + 1;
    let mut best: Vec<Option<Path>> = vec![None; (2 * offset + 1) as usize];
    let at = |diagonal: isize| (diagonal + offset) as usize;
    let mut seed = Path {
        old: -1,
        last: None,
    };
    let n = extract(&mut components, &mut seed, 0);
    let finished = 'search: {
        if seed.old + 1 >= old_len && n + 1 >= new_len {
            break 'search seed.last;
        }
        best[at(0)] = Some(seed);
        let (mut min_diagonal, mut max_diagonal) = (isize::MIN, isize::MAX);
        for edit in 1..=max_edit {
            let mut diagonal = min_diagonal.max(-edit);
            while diagonal <= max_diagonal.min(edit) {
                let remove = best[at(diagonal - 1)].take();
                let add = best[at(diagonal + 1)];
                let can_add = add.is_some_and(|path| (0..new_len).contains(&(path.old - diagonal)));
                let can_remove = remove.is_some_and(|path| path.old + 1 < old_len);
                if !can_add && !can_remove {
                    best[at(diagonal)] = None;
                    diagonal += 2;
                    continue;
                }
                // Branch from the path furthest along the old text; ties remove.
                let adds =
                    !can_remove || (can_add && remove.zip(add).is_some_and(|(r, a)| r.old < a.old));
                let mut path = match (adds, add, remove) {
                    (true, Some(add), _) => add_to_path(&mut components, add, Change::Added, 0),
                    (false, _, Some(remove)) => {
                        add_to_path(&mut components, remove, Change::Removed, 1)
                    }
                    _ => unreachable!("a branch is only taken from an existing path"),
                };
                let n = extract(&mut components, &mut path, diagonal);
                if path.old + 1 >= old_len && n + 1 >= new_len {
                    break 'search path.last;
                }
                best[at(diagonal)] = Some(path);
                if path.old + 1 >= old_len {
                    max_diagonal = max_diagonal.min(diagonal - 1);
                }
                if n + 1 >= new_len {
                    min_diagonal = min_diagonal.max(diagonal + 1);
                }
                diagonal += 2;
            }
        }
        None
    };
    let mut out = Vec::new();
    let mut next = finished;
    while let Some(index) = next {
        let component = components[index];
        out.push((component.count, component.change));
        next = component.previous;
    }
    out.reverse();
    out
}
