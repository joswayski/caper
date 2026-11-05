//! What the UI draws: one open conversation with its space, channels and
//! members. The shared client core (`apps/native/client`) will fill this from
//! the API and gateway; until then `Conversation::demo` generates the
//! benchmark fixture locally (a labelled development mock, never shown as a
//! real account).

pub struct Person {
    pub name: String,
    pub initials: String,
    pub color: u32,
    pub online: bool,
}

pub enum Span {
    Text(String),
    Bold(String),
    Italic(String),
    Link(String),
    /// A member, by index into `Conversation::members`.
    Mention(usize),
}

pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Seed of the generated demo pixels.
    pub seed: u32,
}

pub struct Reaction {
    pub emoji: String,
    pub count: u32,
}

pub struct Message {
    pub id: String,
    pub author: usize,
    pub time: String,
    /// Same author shortly after the previous message: no avatar or name.
    pub grouped: bool,
    pub spans: Vec<Span>,
    pub picture: Option<Picture>,
    pub reactions: Vec<Reaction>,
}

pub struct Channel {
    pub name: String,
    pub private: bool,
}

pub struct Space {
    pub name: String,
}

pub struct Conversation {
    pub spaces: Vec<Space>,
    pub space: usize,
    pub channels: Vec<Channel>,
    pub channel: usize,
    pub members: Vec<Person>,
    /// The first `listed_members` of `members` are the member list; the
    /// rest are authors who are no longer members.
    pub listed_members: usize,
    /// Whether `Person::online` is known (the gateway reports presence).
    pub presence: bool,
    pub messages: Vec<Message>,
    demo: Option<Demo>,
}

impl Conversation {
    pub fn live(
        spaces: Vec<Space>,
        space: usize,
        channels: Vec<Channel>,
        channel: usize,
        members: Vec<Person>,
        messages: Vec<Message>,
    ) -> Self {
        let listed_members = members.len();
        Self {
            spaces,
            space,
            channels,
            channel,
            members,
            listed_members,
            presence: false,
            messages,
            demo: None,
        }
    }

    /// The message's plain text (what selection copies and screen readers
    /// read) and its styled spans as (UTF-16 start, length, kind).
    pub fn plain(&self, i: usize) -> (String, Vec<(usize, usize, SpanKind)>) {
        let mut out = String::new();
        let mut len16 = 0;
        let mut spans = Vec::new();
        for span in &self.messages[i].spans {
            let start = len16;
            let (text, kind) = match span {
                Span::Text(t) => (t.clone(), None),
                Span::Bold(t) => (t.clone(), Some(SpanKind::Bold)),
                Span::Italic(t) => (t.clone(), Some(SpanKind::Italic)),
                Span::Link(u) => (u.clone(), Some(SpanKind::Link)),
                Span::Mention(a) => (
                    format!("@{}", self.members[*a].name),
                    Some(SpanKind::Mention),
                ),
            };
            len16 += text.encode_utf16().count();
            out.push_str(&text);
            if let Some(kind) = kind {
                spans.push((start, len16 - start, kind));
            }
        }
        (out, spans)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanKind {
    Bold,
    Italic,
    Link,
    Mention,
}

// ---------------------------------------------------------------------------
// Development mock: the benchmark fixture (3000 messages, 48 members), the
// same deterministic generator the earlier Win32, egui and GPUI recreations
// used, so numbers stay comparable.

const AUTHORS: usize = 48;
const CHANNELS: usize = 36;
const FIRST: &[&str] = &[
    "Ana", "Bruno", "Chen", "Dalia", "Emeka", "Fatima", "Gus", "Hana", "Ivan", "Jules", "Kai",
    "Lena", "Mateo", "Nia", "Omar", "Priya", "Quinn", "Rosa", "Sam", "Tomás", "Uma", "Viktor",
    "Wren", "Yuki",
];
const LAST: &[&str] = &["Okafor", "Silva", "Nakamura", "Kowalski", "Haddad", "Reyes"];
const WORDS: &[&str] = &[
    "the",
    "voice",
    "channel",
    "is",
    "pretty",
    "good",
    "today",
    "but",
    "we",
    "should",
    "probably",
    "ship",
    "it",
    "after",
    "lunch",
    "anyone",
    "seen",
    "my",
    "headphones",
    "latency",
    "feels",
    "lower",
    "than",
    "yesterday",
    "release",
    "notes",
    "look",
    "great",
    "can",
    "you",
    "check",
    "staging",
    "again",
    "I",
    "think",
    "the",
    "invite",
    "link",
    "expired",
    "already",
    "lol",
    "honestly",
    "that",
    "was",
    "fast",
    "nice",
    "work",
    "everyone",
    "meeting",
    "moved",
    "to",
    "tomorrow",
    "morning",
    "coffee",
    "first",
    "then",
    "bugs",
    "screenshots",
    "attached",
    "below",
    "server",
    "restarted",
    "fine",
    "now",
    "thanks",
    "for",
    "the",
    "quick",
    "fix",
    "who",
    "owns",
    "this",
    "space",
    "game",
    "night",
    "friday",
    "playlist",
    "updated",
    "with",
    "new",
    "songs",
];
const EMOJI: &[&str] = &[
    "😅",
    "🎉",
    "👍",
    "✨",
    "🤔",
    "🚀",
    "🙌🏽",
    "☕",
    "👨‍👩‍👧",
    "🔥",
    "💯",
    "😂",
];
const REACTIONS: &[&str] = &["👍", "🎉", "😂", "🔥", "❤️", "👀"];
const INTERNATIONAL: &[&str] = &[
    "今日のリリースは最高でした！",
    "我们明天早上开会吧",
    "회의는 내일 아침으로 옮겼어요",
    "مرحبا بالجميع، كيف حالكم؟",
    "शाम को मिलते हैं, ठीक है?",
    "Привет всем, сервер снова работает",
    "שלום לכולם, הכול עובד עכשיו",
];
pub const AUTHOR_COLORS: [u32; 6] = [0x637A43, 0xB64D32, 0x4D6FB6, 0x8A5BB0, 0xC29A3A, 0x3A9C9A];

struct Demo {
    rng: u64,
    next: u64,
    minute: u32,
}

impl Demo {
    fn next(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

impl Conversation {
    pub fn demo(count: usize) -> Self {
        let members = (0..AUTHORS)
            .map(|i| {
                let first = FIRST[i % FIRST.len()];
                let last = LAST[(i / FIRST.len() + i) % LAST.len()];
                Person {
                    name: format!("{first} {last}"),
                    initials: format!("{}{}", &first[..1], &last[..1]),
                    color: AUTHOR_COLORS[i % 6],
                    online: i % 3 != 0,
                }
            })
            .collect();
        let names = [
            "general",
            "random",
            "release",
            "voice-lab",
            "design",
            "music",
            "games",
            "support",
            "feedback",
            "photos",
            "off-topic",
            "planning",
        ];
        let channels = (0..CHANNELS)
            .map(|i| Channel {
                name: names[i % 12].to_string()
                    + &if i >= 12 {
                        format!("-{}", i / 12)
                    } else {
                        String::new()
                    },
                private: i % 12 == 11,
            })
            .collect();
        let spaces = [
            "Caper Crew",
            "Friends",
            "Game night",
            "Music",
            "Work",
            "Design",
            "Reading",
            "Photos",
        ]
        .iter()
        .map(|n| Space { name: (*n).into() })
        .collect();
        let mut demo = Demo {
            rng: 0x9E37_79B9_7F4A_7C15,
            next: 0,
            minute: 9 * 60,
        };
        demo.next();
        let mut c = Self {
            spaces,
            space: 0,
            channels,
            channel: 0,
            members,
            listed_members: AUTHORS,
            presence: true,
            messages: Vec::with_capacity(count + 512),
            demo: Some(demo),
        };
        for _ in 0..count {
            c.push_demo();
        }
        c
    }

    /// Appends one generated message (how the mock simulates live arrivals).
    pub fn push_demo(&mut self) {
        let Some(rng) = self.demo.as_mut() else {
            return;
        };
        let previous = self.messages.last().map(|m| m.author);
        let author = match previous {
            Some(p) if rng.chance(45) => p,
            _ => rng.below(AUTHORS as u64) as usize,
        };
        let gap = if rng.chance(70) {
            1
        } else {
            1 + rng.below(12) as u32
        };
        rng.minute += gap;
        let grouped = previous == Some(author) && gap < 5;
        let mut spans = Vec::new();
        let words = match rng.below(10) {
            0..=4 => 3 + rng.below(10),
            5..=7 => 12 + rng.below(25),
            _ => 40 + rng.below(70),
        };
        let mut buffer = String::new();
        let flush = |buffer: &mut String, spans: &mut Vec<Span>| {
            if !buffer.is_empty() {
                spans.push(Span::Text(std::mem::take(buffer)));
            }
        };
        for w in 0..words {
            if w > 0 {
                buffer.push(' ');
            }
            match rng.below(100) {
                0..=2 => {
                    flush(&mut buffer, &mut spans);
                    let (a, b) = (rng.pick(WORDS), rng.pick(WORDS));
                    spans.push(Span::Bold(format!("{a} {b}")));
                }
                3..=4 => {
                    flush(&mut buffer, &mut spans);
                    spans.push(Span::Italic(rng.pick(WORDS).to_string()));
                }
                5 => {
                    flush(&mut buffer, &mut spans);
                    spans.push(Span::Mention(rng.below(AUTHORS as u64) as usize));
                }
                6 => {
                    flush(&mut buffer, &mut spans);
                    let n = rng.below(9000);
                    spans.push(Span::Link(format!("https://caper.chat/s/{n}")));
                }
                7..=9 => buffer.push_str(rng.pick(EMOJI)),
                10 if rng.chance(30) => buffer.push_str(rng.pick(INTERNATIONAL)),
                _ => buffer.push_str(rng.pick(WORDS)),
            }
        }
        flush(&mut buffer, &mut spans);
        let picture = rng.chance(4).then(|| {
            let w = 240 + rng.below(160) as u32;
            Picture {
                width: w,
                height: w * (5 + rng.below(4) as u32) / 10,
                seed: rng.next() as u32,
            }
        });
        let reactions = if rng.chance(12) {
            (0..1 + rng.below(3))
                .map(|i| Reaction {
                    emoji: REACTIONS[(i as usize * 2 + author) % REACTIONS.len()].into(),
                    count: 1 + rng.below(9) as u32,
                })
                .collect()
        } else {
            Vec::new()
        };
        let id = format!("m{:06}", rng.next);
        rng.next += 1;
        let (h, m) = ((rng.minute / 60) % 12, rng.minute % 60);
        let time = format!(
            "{}:{m:02} {}",
            if h == 0 { 12 } else { h },
            if (rng.minute / 60) % 24 < 12 {
                "AM"
            } else {
                "PM"
            }
        );
        self.messages.push(Message {
            id,
            author,
            time,
            grouped,
            spans,
            picture,
            reactions,
        });
    }
}

impl Conversation {
    /// Adds a message from the signed-in member (member 0 in the mock) to
    /// this conversation only; the client core will send it instead.
    pub fn send_local(&mut self, text: String) {
        let time = self
            .messages
            .last()
            .map(|m| m.time.clone())
            .unwrap_or_default();
        let grouped = self.messages.last().is_some_and(|m| m.author == 0);
        let id = format!("local{}", self.messages.len());
        self.messages.push(Message {
            id,
            author: 0,
            time,
            grouped,
            spans: vec![Span::Text(text)],
            picture: None,
            reactions: Vec::new(),
        });
    }
}

/// Procedural premultiplied BGRA pixels for a demo picture.
pub fn demo_pixels(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let (r0, g0, b0) = (seed & 0xff, (seed >> 8) & 0xff, (seed >> 16) & 0xff);
    for y in 0..h {
        for x in 0..w {
            let v = ((x * 255 / w) ^ (y * 255 / h)) & 0xff;
            out.extend_from_slice(&[
                ((b0 + y % 89) / 2) as u8,
                ((g0 + x % 97) / 2) as u8,
                ((r0 + v) / 2) as u8,
                255,
            ]);
        }
    }
    out
}
