//! Link preview cards (docs/media.md "Link embeds"), as web's `Embeds.tsx`:
//! a card per link under a message, drawn from its blocks in order, and the
//! main composer's preview of its first link before sending.
//!
//! Every string is plain text and every link opens in the system browser,
//! like message links. Images and posters are unsigned `embed.caper.chat`
//! URLs loaded through the attachment image cache under [`EMBED_KEY`] ids,
//! so they never ask for a signed-URL refresh. Images and direct videos open
//! in the in-app viewer and player, as attachments do; GIF-style videos loop
//! muted there.
//!
//! Players (YouTube, Vimeo, Twitch) show their poster and a Play button.
//! Play loads Caper's own `play` page (only from `embed.caper.chat` or
//! `embed.staging.caper.chat`) in the operating system's web engine over the
//! box ([`crate::web_player`]); without a usable page, or on a platform
//! without an engine, it opens the card's link or the page in the system
//! browser. Without the engine the box says what to install and offers the
//! browser.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Stroke};

use crate::attachments::{EMBED_KEY, ImageState, Media};
use crate::model::{Attachment, AttachmentKind, AttachmentStatus, Embed, EmbedBlock, EmbedImage};
use crate::web_player::{self, WebPlayer};
use crate::{
    ACTION, ACTION_HOVER, BORDER, MUTED, NavIcon, ON_ACTION, RAISED, SURFACE, TEXT, links, motion,
    paint_icon,
};

/// Media in a card, as web's `fit()`.
pub const MAX_WIDTH: f32 = 400.0;
pub const MAX_HEIGHT: f32 = 300.0;
/// Cards keep to web's `min(432px, 100%)`.
const CARD_WIDTH: f32 = 432.0;
const THUMBNAIL: f32 = 80.0;
/// Web clamps card text to 8 lines.
const TEXT_ROWS: usize = 8;
/// The composer waits this long after the link last changed before asking.
pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(400);

/// Display size that reserves layout space before media loads; media of
/// unknown size gets the full width at 16:9.
pub fn fit(width: Option<u32>, height: Option<u32>) -> egui::Vec2 {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (MAX_WIDTH / width).min(MAX_HEIGHT / height).min(1.0);
            egui::vec2(
                (width * scale).round().max(1.0),
                (height * scale).round().max(1.0),
            )
        }
        _ => egui::vec2(MAX_WIDTH, (MAX_WIDTH * 9.0 / 16.0).round()),
    }
}

/// [`fit`] narrowed to `available`, keeping its aspect ratio.
fn fit_within(size: egui::Vec2, available: f32) -> egui::Vec2 {
    if size.x <= available || available <= 0.0 {
        size
    } else {
        egui::vec2(available, (size.y * available / size.x).round().max(1.0))
    }
}

/// The first link in a draft that will get a card, for the composer preview
/// (web `previewLink`): links written `<like this>` get none.
pub fn preview_link(draft: &str) -> Option<String> {
    let segments = links::segments(draft);
    segments.iter().enumerate().find_map(|(index, segment)| {
        let href = segment.href.as_ref()?;
        let bracketed = index
            .checked_sub(1)
            .and_then(|previous| segments.get(previous))
            .is_some_and(|previous| previous.text.ends_with('<'))
            && segments
                .get(index + 1)
                .is_some_and(|next| next.text.starts_with('>'));
        (!bracketed).then(|| href.clone())
    })
}

/// A player block's `play` page, only when it is HTTPS on Caper's embed
/// host with no credentials or port, under `/play/`.
/// Shared with the web view's own navigation rules.
pub fn play_page(url: &str) -> Option<url::Url> {
    caper_web_player::play_page(url)
}

/// What Play opens: the `play` page when it is Caper's, else the card's link.
pub fn play_target(embed: &Embed, play: Option<&str>) -> String {
    play.and_then(play_page)
        .map_or_else(|| embed.url.clone(), |page| page.to_string())
}

/// A link that may open externally: http(s) only, as message links.
fn openable(url: &str) -> bool {
    caper_web_player::openable(url)
}

fn key(url: &str) -> String {
    format!("{EMBED_KEY}{url}")
}

/// An image block's items as viewer files: unsigned, ready, sized when known.
fn image_files(items: &[EmbedImage], name: &str) -> Vec<Attachment> {
    items
        .iter()
        .map(|image| Attachment {
            id: key(&image.url),
            spoiler: false,
            kind: AttachmentKind::Image,
            // The decoders sniff the real format; this only picks the path.
            content_type: "image/jpeg".into(),
            name: image
                .alt
                .clone()
                .filter(|alt| !alt.is_empty())
                .unwrap_or_else(|| name.into()),
            size: 0,
            width: image.width,
            height: image.height,
            duration_ms: None,
            preview: true,
            url: Some(image.url.clone()),
            preview_url: Some(image.url.clone()),
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
        })
        .collect()
}

/// A direct video as a viewer file; its poster shares the inline tile's
/// texture (both use the video's id).
fn video_file(block: &EmbedBlock, name: &str) -> Option<Attachment> {
    let EmbedBlock::Video {
        url,
        poster,
        width,
        height,
        duration_ms,
        gif,
    } = block
    else {
        return None;
    };
    Some(Attachment {
        id: key(url),
        spoiler: false,
        kind: AttachmentKind::Video,
        content_type: if url.to_ascii_lowercase().contains(".webm") {
            "video/webm"
        } else {
            "video/mp4"
        }
        .into(),
        name: name.into(),
        size: 0,
        width: *width,
        height: *height,
        duration_ms: *duration_ms,
        preview: poster.is_some(),
        url: Some(url.clone()),
        preview_url: poster.clone(),
        unavailable: false,
        status: AttachmentStatus::Ready,
        animated: *gif,
    })
}

/// The card's name for players, viewers and screen readers: its title, else
/// its author, else its site.
fn heading(embed: &Embed) -> &str {
    embed
        .blocks
        .iter()
        .find_map(|block| match block {
            EmbedBlock::Title { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .or_else(|| {
            embed.blocks.iter().find_map(|block| match block {
                EmbedBlock::Author { name, .. } => Some(name.as_str()),
                _ => None,
            })
        })
        .unwrap_or(&embed.site.name)
}

/// Web's `dateStyle: "medium"`, in local time.
fn footer_date(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d, %Y")
                .to_string()
        })
}

pub fn footer_line(text: Option<&str>, timestamp: Option<&str>) -> String {
    [
        text.filter(|text| !text.is_empty()).map(str::to_owned),
        timestamp.and_then(footer_date),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// What a card asks the app to do.
pub enum Action {
    /// Open an http(s) link in the system browser.
    Open(String),
    /// Show these files in the viewer, starting at the index.
    View(Vec<Attachment>, usize),
}

#[derive(Default)]
pub struct Shown {
    pub action: Option<Action>,
    /// A card link or control is hovered, so the row's own menu stays closed.
    pub hovered: bool,
    /// The composer preview's × was pressed.
    pub removed: bool,
}

/// A message's cards. `scope` keeps their ids apart (the message id);
/// `arrival` is the entrance progress (1 when settled); `removable` adds the
/// composer's × to the first card.
pub fn cards(
    ui: &mut egui::Ui,
    media: &mut Media,
    player: &mut WebPlayer,
    scope: &str,
    embeds: &[Embed],
    arrival: f32,
    removable: bool,
) -> Shown {
    let mut shown = Shown::default();
    if embeds.is_empty() {
        return shown;
    }
    motion::arrive(ui, arrival, egui::vec2(0.0, 4.0), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
        // Rows hug their text rather than the app's 36px control height.
        ui.spacing_mut().interact_size.y = 20.0;
        let width = ui.available_width().min(CARD_WIDTH);
        for (index, embed) in embeds.iter().enumerate() {
            let remove = removable && index == 0;
            card(
                ui,
                media,
                player,
                (scope, index),
                embed,
                width,
                remove,
                &mut shown,
            );
        }
    });
    if media.busy() {
        ui.ctx().request_repaint();
    }
    shown
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    media: &mut Media,
    player: &mut WebPlayer,
    id: (&str, usize),
    embed: &Embed,
    width: f32,
    remove: bool,
    shown: &mut Shown,
) {
    let thumbnail = embed.blocks.iter().find_map(|block| match block {
        EmbedBlock::Images {
            items,
            thumbnail: true,
        } => items.first(),
        _ => None,
    });
    // Room for the remove button over the card's corner.
    let right = if remove { 36 } else { 12 };
    let frame = egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8)
        .inner_margin(egui::Margin {
            left: 12,
            right,
            top: 10,
            bottom: 12,
        });
    let inner = (width - 12.0 - f32::from(right) - 2.0).max(40.0);
    let response = frame.show(ui, |ui| {
        ui.set_width(inner);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let body = if thumbnail.is_some() {
                (inner - THUMBNAIL - 12.0).max(40.0)
            } else {
                inner
            };
            ui.allocate_ui_with_layout(
                egui::vec2(body, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(body);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    site_line(ui, id, embed, shown);
                    for (block_index, block) in embed.blocks.iter().enumerate() {
                        let block_id = (id.0, id.1, block_index);
                        draw_block(ui, media, player, block_id, embed, block, body, shown);
                    }
                },
            );
            if let Some(image) = thumbnail {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(THUMBNAIL, THUMBNAIL), egui::Sense::click());
                let state = media.image(&key(&image.url), Some(&image.url));
                tile(ui, rect, CornerRadius::same(6), state, &response, None);
                let label = image.alt.clone().unwrap_or_else(|| embed.site.name.clone());
                if clicked(&response, &label, shown) {
                    shown.action = Some(Action::Open(embed.url.clone()));
                }
            }
        });
    });
    if remove {
        let rect = egui::Rect::from_min_size(
            response.response.rect.right_top() + egui::vec2(-28.0, 4.0),
            egui::vec2(24.0, 24.0),
        );
        let button = ui.interact(
            rect,
            ui.id().with(("embed-remove", id.0)),
            egui::Sense::click(),
        );
        let hot = button.hovered() || button.has_focus();
        ui.painter().rect(
            rect,
            8,
            RAISED,
            Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
        paint_icon(
            ui.painter(),
            rect.shrink(6.0),
            NavIcon::Close,
            if hot { TEXT } else { MUTED },
        );
        if clicked(&button, "Remove link preview", shown) {
            shown.removed = true;
        }
    }
}

/// Reports a control's name and pointer state; returns whether it was clicked.
fn clicked(response: &egui::Response, label: &str, shown: &mut Shown) -> bool {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.has_focus() {
        response.ctx.layer_painter(response.layer_id).rect_stroke(
            response.rect.expand(2.0),
            8,
            Stroke::new(2.0, crate::TERRACOTTA_BRIGHT),
            egui::StrokeKind::Outside,
        );
    }
    shown.hovered |= response.hovered();
    let response = response
        .clone()
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.clicked()
}

/// Text that opens `url` when it has one: underlined while hovered, like
/// web's card links.
#[allow(clippy::too_many_arguments)]
fn text_link(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &str,
    font: egui::FontId,
    color: Color32,
    url: Option<&str>,
    wrap: bool,
    shown: &mut Shown,
) -> bool {
    let url = url.filter(|url| openable(url));
    let hot = url.is_some() && ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let mut rich = egui::RichText::new(text).font(font).color(color);
    if hot {
        rich = rich.underline();
    }
    let label = egui::Label::new(rich).selectable(false);
    let label = if wrap { label.wrap() } else { label.truncate() };
    let response = if url.is_some() {
        ui.add(label.sense(egui::Sense::click()))
    } else {
        ui.add(label)
    };
    let Some(url) = url else {
        return false;
    };
    let now = response.hovered() || response.has_focus();
    if now != hot {
        ui.data_mut(|data| data.insert_temp(id, now));
        ui.ctx().request_repaint();
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, text));
    shown.hovered |= response.hovered();
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.context_menu(|ui| {
        if ui.button("Open link").clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            ui.close();
        }
        if ui.button("Copy link").clicked() {
            ui.ctx().copy_text(url.to_owned());
            ui.close();
        }
    });
    if response.clicked() {
        shown.action = Some(Action::Open(url.to_owned()));
    }
    response.clicked()
}

/// The muted site line; the domain always shows, so a page cannot pass
/// itself off as another site.
fn site_line(ui: &mut egui::Ui, id: (&str, usize), embed: &Embed, shown: &mut Shown) {
    let text = if embed.site.name == embed.site.domain {
        embed.site.name.clone()
    } else {
        format!("{}  {}", embed.site.name, embed.site.domain)
    };
    text_link(
        ui,
        egui::Id::new(("embed-site", id)),
        &text,
        egui::FontId::proportional(11.5),
        MUTED,
        Some(&embed.url),
        false,
        shown,
    );
}

fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("Satoshi Bold".into()))
}

#[allow(clippy::too_many_arguments)]
fn draw_block(
    ui: &mut egui::Ui,
    media: &mut Media,
    player: &mut WebPlayer,
    id: (&str, usize, usize),
    embed: &Embed,
    block: &EmbedBlock,
    width: f32,
    shown: &mut Shown,
) {
    match block {
        EmbedBlock::Author {
            name,
            handle,
            url,
            icon,
        } => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(icon) = icon {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    match media.image(&key(icon), Some(icon)) {
                        ImageState::Ready(texture) => {
                            cover(ui, rect, texture, CornerRadius::same(10));
                        }
                        _ => {
                            ui.painter().circle_filled(rect.center(), 10.0, RAISED);
                        }
                    }
                }
                ui.spacing_mut().item_spacing.x = 6.0;
                text_link(
                    ui,
                    egui::Id::new(("embed-author", id)),
                    name,
                    bold(13.0),
                    TEXT,
                    url.as_deref(),
                    false,
                    shown,
                );
                if let Some(handle) = handle.as_deref().filter(|handle| !handle.is_empty()) {
                    text_link(
                        ui,
                        egui::Id::new(("embed-handle", id)),
                        handle,
                        egui::FontId::proportional(13.0),
                        MUTED,
                        url.as_deref(),
                        false,
                        shown,
                    );
                }
            });
        }
        EmbedBlock::Title { text, url } => {
            text_link(
                ui,
                egui::Id::new(("embed-title", id)),
                text,
                bold(14.4),
                TEXT,
                url.as_deref(),
                true,
                shown,
            );
        }
        EmbedBlock::Text { text } => {
            let body = crate::rich_text_rows(
                ui,
                &format!("embed-{}-{}-{}", id.0, id.1, id.2),
                text,
                egui::TextFormat::simple(egui::FontId::proportional(13.6), TEXT),
                &[],
                |_| String::new(),
                false,
                TEXT_ROWS,
            );
            shown.hovered |= body.link_hovered;
        }
        EmbedBlock::Images {
            thumbnail: true, ..
        } => {}
        EmbedBlock::Images { items, .. } => {
            let rects = gallery(items, width);
            let origin = ui.next_widget_position();
            let bottom = rects.iter().map(|rect| rect.bottom()).fold(0.0, f32::max);
            let (_, _) = ui.allocate_exact_size(egui::vec2(width, bottom), egui::Sense::hover());
            for (index, (image, rect)) in items.iter().zip(rects).enumerate() {
                let rect = rect.translate(origin.to_vec2());
                let response = ui.interact(
                    rect,
                    ui.id().with(("embed-image", id, index)),
                    egui::Sense::click(),
                );
                let state = media.image(&key(&image.url), Some(&image.url));
                tile(ui, rect, CornerRadius::same(6), state, &response, None);
                let label = format!(
                    "View {}",
                    image
                        .alt
                        .as_deref()
                        .filter(|alt| !alt.is_empty())
                        .unwrap_or("image")
                );
                if clicked(&response, &label, shown) {
                    shown.action = Some(Action::View(image_files(items, heading(embed)), index));
                }
            }
        }
        EmbedBlock::Video {
            url,
            poster,
            width: natural_width,
            height,
            gif,
            ..
        } => {
            let size = fit_within(fit(*natural_width, *height), width);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            let state = poster.as_deref().map_or(ImageState::Failed, |poster| {
                media.image(&key(url), Some(poster))
            });
            tile(
                ui,
                rect,
                CornerRadius::same(6),
                state,
                &response,
                Some(if *gif { Badge::Gif } else { Badge::Play }),
            );
            let label = if *gif {
                format!("Play animation {}", heading(embed))
            } else {
                format!("Play {}", heading(embed))
            };
            if clicked(&response, &label, shown)
                && let Some(file) = video_file(block, heading(embed))
            {
                shown.action = Some(Action::View(vec![file], 0));
            }
        }
        EmbedBlock::Player {
            provider,
            play,
            poster,
            width: natural_width,
            height,
            ..
        } => {
            let size = fit_within(fit(*natural_width, *height), width);
            let state = poster.as_deref().map_or(ImageState::Failed, |poster| {
                media.image(&key(poster), Some(poster))
            });
            let player_key = web_player::Key {
                scope: id.0.to_owned(),
                card: id.1,
                block: id.2,
            };
            let Some(status) = player.status(&player_key) else {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                tile(
                    ui,
                    rect,
                    CornerRadius::same(6),
                    state,
                    &response,
                    Some(Badge::Player),
                );
                if clicked(&response, &format!("Play {}", heading(embed)), shown) {
                    match play
                        .as_deref()
                        .and_then(play_page)
                        .filter(|_| web_player::SUPPORTED)
                    {
                        Some(page) => player.start(ui.ctx(), player_key, page, provider),
                        None => {
                            shown.action = Some(Action::Open(play_target(embed, play.as_deref())));
                        }
                    }
                }
                return;
            };
            // The web view covers this box while it can; underneath (and
            // whenever it hides) the poster stays, so nothing moves.
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
            tile(ui, rect, CornerRadius::same(6), state, &response, None);
            ui.painter()
                .rect_filled(rect, 6, Color32::from_black_alpha(150));
            let name = caper_web_player::provider_name(provider);
            // Laid out this frame (the view's place, and the player stays).
            player.place(&player_key, rect, ui.clip_rect(), ui.layer_id());
            match status {
                web_player::Status::Starting | web_player::Status::Inline => {
                    playing_badge(ui, rect, status == web_player::Status::Starting);
                }
                web_player::Status::Window => {
                    match player_notice(
                        ui,
                        rect,
                        (id.0, id.1, id.2),
                        &format!("Playing in a Caper window ({name})"),
                        &["Show window", "Close"],
                        shown,
                    ) {
                        Some(0) => player.raise(),
                        Some(_) => player.stop(),
                        None => {}
                    }
                }
                web_player::Status::Unavailable(why) => {
                    if player_notice(
                        ui,
                        rect,
                        (id.0, id.1, id.2),
                        why.message(),
                        &["Open in browser"],
                        shown,
                    )
                    .is_some()
                    {
                        shown.action = Some(Action::Open(play_target(embed, play.as_deref())));
                    }
                }
            }
        }
        EmbedBlock::Footer { text, timestamp } => {
            let line = footer_line(text.as_deref(), timestamp.as_deref());
            if !line.is_empty() {
                ui.add(
                    egui::Label::new(egui::RichText::new(line).size(11.5).color(MUTED))
                        .selectable(false)
                        .wrap(),
                );
            }
        }
    }
}

/// The small mark over a poster whose video plays in the web view (seen
/// while the view hides under a menu or at the timeline's edge).
fn playing_badge(ui: &egui::Ui, rect: egui::Rect, starting: bool) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let font = bold(12.0);
    let label = painter.layout_no_wrap("Playing".into(), font, TEXT);
    let spinner = if starting { 18.0 } else { 0.0 };
    let size = egui::vec2(label.size().x + 24.0 + spinner, 28.0);
    let badge = egui::Rect::from_center_size(rect.center(), size);
    painter.rect_filled(badge, 8, Color32::from_black_alpha(200));
    if starting {
        egui::Spinner::new().size(12.0).color(MUTED).paint_at(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(badge.left() + 18.0, badge.center().y),
                egui::vec2(12.0, 12.0),
            ),
        );
    }
    painter.galley(
        egui::pos2(
            badge.left() + 12.0 + spinner,
            badge.center().y - label.size().y / 2.0,
        ),
        label,
        TEXT,
    );
}

/// A short message centred over the poster with buttons under it (the first
/// is the primary action); returns the one clicked.
fn player_notice(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: (&str, usize, usize),
    text: &str,
    buttons: &[&str],
    shown: &mut Shown,
) -> Option<usize> {
    let area = rect.shrink2(egui::vec2(16.0, 12.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("embed-player-notice", id))
            .max_rect(area)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let text_height = child
        .painter()
        .layout(
            text.to_owned(),
            egui::FontId::proportional(13.0),
            TEXT,
            area.width(),
        )
        .size()
        .y;
    let height = text_height + 10.0 + 36.0;
    child.add_space(((area.height() - height) / 2.0).max(0.0));
    child.add(
        egui::Label::new(egui::RichText::new(text).size(13.0).color(TEXT))
            .selectable(false)
            .wrap(),
    );
    child.add_space(10.0);
    let mut pressed = None;
    // Centred: the dialog buttons' 12px padding either side of bold 12px text.
    let row = buttons
        .iter()
        .map(|label| {
            child
                .painter()
                .layout_no_wrap((*label).to_owned(), bold(12.0), TEXT)
                .size()
                .x
                + 24.0
        })
        .sum::<f32>()
        + 8.0 * buttons.len().saturating_sub(1) as f32;
    child.allocate_ui_with_layout(
        egui::vec2(row.min(area.width()), 36.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (index, label) in buttons.iter().enumerate() {
                let response = if index == 0 {
                    crate::primary_button(ui, label, true)
                } else {
                    crate::secondary_button(ui, label, true)
                };
                shown.hovered |= response.hovered();
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    pressed = Some(index);
                }
            }
        },
    );
    pressed
}

/// Gallery slots relative to the gallery's top-left corner, as web's grid:
/// one image at its fitted size; two side by side 220 high; three with the
/// first spanning both rows; four in two rows of 148.
pub fn gallery(items: &[EmbedImage], width: f32) -> Vec<egui::Rect> {
    const GAP: f32 = 4.0;
    let half = ((width - GAP) / 2.0).max(1.0);
    let cell = |column: f32, row: f32, height: f32, rows: f32| {
        egui::Rect::from_min_size(
            egui::pos2(column * (half + GAP), row * (height + GAP)),
            egui::vec2(half, height * rows + GAP * (rows - 1.0)),
        )
    };
    match items {
        [] => Vec::new(),
        [image] => vec![egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            fit_within(fit(image.width, image.height), width),
        )],
        [_, _] => vec![cell(0.0, 0.0, 220.0, 1.0), cell(1.0, 0.0, 220.0, 1.0)],
        [_, _, _] => vec![
            cell(0.0, 0.0, 148.0, 2.0),
            cell(1.0, 0.0, 148.0, 1.0),
            cell(1.0, 1.0, 148.0, 1.0),
        ],
        _ => (0..items.len().min(4))
            .map(|index| cell((index % 2) as f32, (index / 2) as f32, 148.0, 1.0))
            .collect(),
    }
}

/// Paints `texture` cropped to fill `rect` (CSS `object-fit: cover`).
fn cover(ui: &egui::Ui, rect: egui::Rect, texture: egui::TextureId, radius: CornerRadius) {
    let source = ui
        .ctx()
        .tex_manager()
        .read()
        .meta(texture)
        .map(|meta| meta.size)
        .unwrap_or([1, 1]);
    let aspect = source[0] as f32 / source[1].max(1) as f32;
    let target = rect.width() / rect.height().max(1.0);
    let uv = if aspect > target {
        egui::vec2(target / aspect, 1.0)
    } else {
        egui::vec2(1.0, aspect / target)
    };
    egui::Image::new((texture, rect.size()))
        .uv(egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv))
        .corner_radius(radius)
        .paint_at(ui, rect);
}

#[derive(Clone, Copy)]
enum Badge {
    Play,
    Gif,
    Player,
}

/// A media tile: the image cropped to cover, a placeholder until it loads,
/// and the video or player mark over it.
fn tile(
    ui: &egui::Ui,
    rect: egui::Rect,
    radius: CornerRadius,
    state: ImageState,
    response: &egui::Response,
    badge: Option<Badge>,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius, RAISED);
    if let ImageState::Ready(texture) = state {
        cover(ui, rect, texture, radius);
    } else if matches!(state, ImageState::Loading) {
        egui::Spinner::new().size(18.0).color(MUTED).paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)),
        );
    }
    let hot = response.hovered() || response.has_focus();
    match badge {
        Some(Badge::Gif) => {
            let badge =
                egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), egui::vec2(34.0, 20.0));
            painter.rect_filled(badge, 6.0, Color32::from_black_alpha(190));
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "GIF",
                bold(11.0),
                TEXT,
            );
        }
        Some(Badge::Play) => {
            let center = rect.center();
            painter.circle_filled(center, 22.0, Color32::from_black_alpha(170));
            paint_icon(
                &painter,
                egui::Rect::from_center_size(center + egui::vec2(1.5, 0.0), egui::vec2(20.0, 20.0)),
                NavIcon::Play,
                TEXT,
            );
        }
        Some(Badge::Player) => {
            // Web's eggshell 52px Play.
            let center = rect.center();
            painter.circle_filled(
                center + egui::vec2(0.0, 2.0),
                28.0,
                Color32::from_black_alpha(90),
            );
            painter.circle_filled(center, 26.0, if hot { ACTION_HOVER } else { ACTION });
            paint_icon(
                &painter,
                egui::Rect::from_center_size(center + egui::vec2(1.5, 0.0), egui::vec2(22.0, 22.0)),
                NavIcon::Play,
                ON_ACTION,
            );
        }
        None => {}
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, if hot { MUTED } else { BORDER }),
        egui::StrokeKind::Inside,
    );
}

/// The main composer's card for its first link (web `useLinkPreview`).
/// Removing it means the message is sent without previews; the choice lasts
/// while that link stays in the draft, and a sent or cleared draft starts
/// over.
#[derive(Default)]
pub struct ComposerPreview {
    /// The draft's preview link and when it became that link.
    link: Option<(String, Instant)>,
    /// The link last asked about.
    asked: Option<String>,
    /// The newest answer: a link and its card, if it has one.
    shown: Option<(String, Option<Embed>)>,
    removed: Option<String>,
    /// The request in flight; replacing it drops a stale answer.
    pending: Option<Receiver<(String, Option<Embed>)>>,
}

impl ComposerPreview {
    /// Track the draft; returns a link to ask about now, or how long until
    /// one is due. Previews need a signed-in account.
    pub fn due(
        &mut self,
        draft: &str,
        signed_in: bool,
        now: Instant,
    ) -> (Option<String>, Option<Duration>) {
        if draft.is_empty() {
            self.removed = None;
        }
        let link = signed_in.then(|| preview_link(draft)).flatten();
        if self.link.as_ref().map(|(link, _)| link) != link.as_ref() {
            self.link = link.map(|link| (link, now));
        }
        let Some((link, since)) = &self.link else {
            return (None, None);
        };
        if self.removed.as_ref() == Some(link) || self.asked.as_ref() == Some(link) {
            return (None, None);
        }
        let waited = now.saturating_duration_since(*since);
        if waited < PREVIEW_DEBOUNCE {
            return (None, Some(PREVIEW_DEBOUNCE - waited));
        }
        self.asked = Some(link.clone());
        (Some(link.clone()), None)
    }

    /// An answer for `url` arrived.
    pub fn answer(&mut self, url: String, embed: Option<Embed>) {
        self.shown = Some((url, embed));
    }

    /// The card to show for the draft's current link.
    pub fn card(&self) -> Option<&Embed> {
        let (link, _) = self.link.as_ref()?;
        if self.removed.as_ref() == Some(link) {
            return None;
        }
        self.shown
            .as_ref()
            .filter(|(url, _)| url == link)
            .and_then(|(_, embed)| embed.as_ref())
    }

    /// Call on every pass with the main draft and the signed-in account's
    /// token. Without `api` (fixtures) nothing is fetched.
    pub fn update(
        &mut self,
        context: &egui::Context,
        draft: &str,
        token: Option<&str>,
        api: Option<&crate::api::Api>,
    ) -> Option<&Embed> {
        if let Some(receiver) = &self.pending
            && let Ok((url, embed)) = receiver.try_recv()
        {
            self.pending = None;
            self.answer(url, embed);
        }
        match self.due(draft, token.is_some(), Instant::now()) {
            (Some(url), _) => {
                if let (Some(api), Some(token)) = (api, token) {
                    let (sender, receiver) = mpsc::channel();
                    let (api, token, context) = (api.clone(), token.to_owned(), context.clone());
                    std::thread::spawn(move || {
                        let embed = api.embed_preview(&token, &url);
                        let _ = sender.send((url, embed));
                        context.request_repaint();
                    });
                    self.pending = Some(receiver);
                }
            }
            (None, Some(wait)) => context.request_repaint_after(wait),
            (None, None) => {}
        }
        self.card()
    }

    pub fn remove(&mut self) {
        self.removed = self.link.as_ref().map(|(link, _)| link.clone());
    }

    /// Whether `text` goes without previews: its preview link was removed.
    pub fn removed(&self, text: &str) -> bool {
        self.removed
            .as_ref()
            .is_some_and(|removed| preview_link(text).as_ref() == Some(removed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[derive(serde::Deserialize)]
    struct Cases {
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        note: String,
        embed: Value,
        blocks: Option<Vec<String>>,
        #[serde(default, rename = "imageCounts")]
        image_counts: Option<Vec<usize>>,
    }

    fn kind(block: &EmbedBlock) -> &'static str {
        match block {
            EmbedBlock::Author { .. } => "author",
            EmbedBlock::Title { .. } => "title",
            EmbedBlock::Text { .. } => "text",
            EmbedBlock::Images { .. } => "images",
            EmbedBlock::Video { .. } => "video",
            EmbedBlock::Player { .. } => "player",
            EmbedBlock::Footer { .. } => "footer",
        }
    }

    #[test]
    fn shared_cases_keep_exactly_the_listed_blocks_or_drop_the_card() {
        let cases: Cases =
            serde_json::from_str(include_str!("../../../../shared/messages/embed-cases.json"))
                .expect("valid shared embed cases");
        assert!(cases.cases.len() >= 8, "the shared cases loaded");
        for case in cases.cases {
            let parsed = Embed::parse(&case.embed);
            let blocks = parsed
                .as_ref()
                .map(|embed| embed.blocks.iter().map(kind).collect::<Vec<_>>());
            assert_eq!(
                blocks,
                case.blocks
                    .as_ref()
                    .map(|blocks| blocks.iter().map(String::as_str).collect()),
                "{}",
                case.note
            );
            if let Some(counts) = case.image_counts {
                let actual: Vec<_> = parsed
                    .iter()
                    .flat_map(|embed| &embed.blocks)
                    .filter_map(|block| match block {
                        EmbedBlock::Images { items, .. } => Some(items.len()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(actual, counts, "{}", case.note);
            }
            // What parses round-trips through the stored shape.
            if let Some(embed) = parsed {
                let value = serde_json::to_value(&embed).unwrap();
                assert_eq!(Embed::parse(&value).as_ref(), Some(&embed), "{}", case.note);
            }
        }
    }

    #[test]
    fn messages_keep_their_cards_and_never_fail_on_malformed_ones() {
        let message = |embeds: Value| {
            serde_json::from_value::<crate::model::Message>(json!({
                "id":"m1","channelId":"c1","seq":"1","createdAt":"2026-10-20T12:00:00Z",
                "clientMessageId":"k","author":{"id":"u","name":"U","isGuest":false},
                "content":{"version":1,"type":"text","text":"https://x.com/a",
                    "embeds":embeds,"suppressEmbeds":"yes"}
            }))
            .expect("a message with odd cards still parses")
        };
        let good = json!({"version":1,"url":"https://x.com/a","site":{"name":"X","domain":"x.com"},
            "blocks":[{"type":"text","text":"a"}]});
        let parsed = message(json!([good, {"version":1}, "card", null]));
        assert_eq!(
            parsed.content.embeds.len(),
            1,
            "malformed cards are skipped"
        );
        assert!(!parsed.content.suppress_embeds, "only `true` suppresses");
        assert!(message(json!("not a list")).content.embeds.is_empty());
        assert!(message(json!(null)).content.embeds.is_empty());
        let encoded = serde_json::to_value(&parsed).unwrap();
        assert_eq!(encoded["content"]["embeds"][0], good);
        assert!(encoded["content"].get("suppressEmbeds").is_none());
        let suppressed: crate::model::Message = serde_json::from_value(json!({
            "id":"m1","channelId":"c1","seq":"1","createdAt":"2026-10-20T12:00:00Z",
            "clientMessageId":"k","author":{"id":"u","name":"U","isGuest":false},
            "content":{"version":1,"type":"text","text":"a","suppressEmbeds":true}
        }))
        .unwrap();
        assert!(suppressed.content.suppress_embeds);
    }

    #[test]
    fn player_and_video_fields_are_checked_like_web() {
        let card = |block: Value| {
            Embed::parse(&json!({"version":1,"url":"https://youtu.be/a",
                "site":{"name":"YouTube","domain":"youtu.be"},"blocks":[block]}))
        };
        assert!(card(json!({"type":"player","provider":"youtube","id":"a","width":0})).is_none());
        assert!(
            card(json!({"type":"player","provider":"youtube","id":"a","poster":"ftp://a"}))
                .is_none()
        );
        assert!(card(json!({"type":"video","url":"https://a/v.mp4","durationMs":"1"})).is_none());
        assert!(card(json!({"type":"author","name":"a","handle":null})).is_none());
        assert!(card(json!({"type":"footer","text":""})).is_none());
        assert!(card(json!({"type":"footer","timestamp":"2026-10-20T12:00:00Z"})).is_some());
        let Some(Embed { blocks, .. }) = card(json!({"type":"player","provider":"vimeo","id":"7",
            "play":"javascript:alert(1)","width":640.4,"height":360,"start":12.9}))
        else {
            panic!("player parses")
        };
        assert_eq!(
            blocks[0],
            EmbedBlock::Player {
                provider: "vimeo".into(),
                id: "7".into(),
                play: None,
                poster: None,
                width: Some(640),
                height: Some(360),
                start: Some(12),
            }
        );
        let images = card(json!({"type":"images","items":[
            {"url":"https://e/1"},{"url":"https://e/2"},{"url":"https://e/3"},
            {"url":"https://e/4"},{"url":"https://e/5"}]}))
        .unwrap();
        assert!(
            matches!(&images.blocks[0], EmbedBlock::Images { items, thumbnail: false } if items.len() == 4)
        );
    }

    #[test]
    fn preview_link_is_the_first_link_not_written_in_angle_brackets() {
        assert_eq!(preview_link("no links here"), None);
        assert_eq!(
            preview_link("see https://x.com/a and https://b.example/c"),
            Some("https://x.com/a".into())
        );
        assert_eq!(
            preview_link("skip <https://x.com/a> but https://b.example/c"),
            Some("https://b.example/c".into())
        );
        assert_eq!(preview_link("<https://x.com/a>"), None);
        assert_eq!(
            preview_link("<https://x.com/a and more"),
            Some("https://x.com/a".into()),
            "only a link closed by `>` is skipped"
        );
        assert_eq!(
            preview_link("www.example.com"),
            Some("https://www.example.com".into())
        );
    }

    #[test]
    fn composer_preview_waits_for_the_link_to_settle_and_remembers_removal() {
        let start = Instant::now();
        let mut preview = ComposerPreview::default();
        assert_eq!(
            preview.due("hi https://x.com/a", false, start),
            (None, None),
            "signed out"
        );
        let (url, wait) = preview.due("hi https://x.com/a", true, start);
        assert_eq!((url, wait), (None, Some(PREVIEW_DEBOUNCE)));
        // Typing after the link does not restart the wait; changing it does.
        let later = start + Duration::from_millis(300);
        assert_eq!(preview.due("hi https://x.com/a ok", true, later).0, None);
        let (url, _) = preview.due("hi https://x.com/a ok", true, start + PREVIEW_DEBOUNCE);
        assert_eq!(url.as_deref(), Some("https://x.com/a"));
        assert_eq!(
            preview.due("hi https://x.com/a ok", true, start + PREVIEW_DEBOUNCE * 2),
            (None, None),
            "asked once"
        );
        assert!(preview.card().is_none());
        let embed = Embed::parse(&json!({"version":1,"url":"https://x.com/a",
            "site":{"name":"X","domain":"x.com"},"blocks":[{"type":"text","text":"a"}]}));
        preview.answer("https://x.com/a".into(), embed.clone());
        assert_eq!(preview.card(), embed.as_ref());
        // An answer for an older link is not shown for the new one.
        let moved = start + PREVIEW_DEBOUNCE * 3;
        assert_eq!(
            preview.due("https://b.example/c", true, moved).1,
            Some(PREVIEW_DEBOUNCE)
        );
        assert!(preview.card().is_none());
        preview.remove();
        assert!(preview.removed("look https://b.example/c"));
        assert!(!preview.removed("look https://x.com/a"));
        assert_eq!(
            preview.due("https://b.example/c", true, moved + PREVIEW_DEBOUNCE),
            (None, None),
            "a removed link is not asked about"
        );
        // A sent or cleared draft starts over.
        preview.due("", true, moved + PREVIEW_DEBOUNCE);
        assert!(!preview.removed("https://b.example/c"));
    }

    #[test]
    fn play_pages_load_only_from_caper_embed_hosts() {
        for good in [
            "https://embed.caper.chat/play/youtube/dQw4w9WgXcQ",
            "https://embed.staging.caper.chat/play/twitch/clip:abc",
        ] {
            assert!(play_page(good).is_some(), "{good}");
        }
        for bad in [
            "http://embed.caper.chat/play/youtube/a",
            "https://embed.caper.chat.evil.example/play/youtube/a",
            "https://user@embed.caper.chat/play/youtube/a",
            "https://embed.caper.chat:8443/play/youtube/a",
            "https://embed.caper.chat/m/sig/i",
            "https://www.youtube.com/embed/a",
            "javascript:alert(1)",
        ] {
            assert!(play_page(bad).is_none(), "{bad}");
        }
        let embed = Embed::parse(&json!({"version":1,"url":"https://youtu.be/a",
            "site":{"name":"YouTube","domain":"youtu.be"},
            "blocks":[{"type":"player","provider":"youtube","id":"a"}]}))
        .unwrap();
        assert_eq!(play_target(&embed, None), "https://youtu.be/a");
        assert_eq!(
            play_target(&embed, Some("https://evil.example/play/youtube/a")),
            "https://youtu.be/a"
        );
        assert_eq!(
            play_target(&embed, Some("https://embed.caper.chat/play/youtube/a")),
            "https://embed.caper.chat/play/youtube/a"
        );
    }

    #[test]
    fn media_fits_like_web_and_galleries_keep_to_the_card() {
        assert_eq!(fit(None, Some(10)), egui::vec2(400.0, 225.0));
        assert_eq!(fit(Some(1280), Some(720)), egui::vec2(400.0, 225.0));
        assert_eq!(fit(Some(300), Some(900)), egui::vec2(100.0, 300.0));
        assert_eq!(fit(Some(200), Some(100)), egui::vec2(200.0, 100.0));
        assert_eq!(
            fit_within(egui::vec2(400.0, 225.0), 200.0),
            egui::vec2(200.0, 113.0)
        );
        let image = |n: usize| {
            (0..n)
                .map(|index| EmbedImage {
                    url: format!("https://e/{index}"),
                    width: None,
                    height: None,
                    alt: None,
                })
                .collect::<Vec<_>>()
        };
        let three = gallery(&image(3), 404.0);
        assert_eq!(
            three[0].height(),
            300.0,
            "the first of three spans both rows"
        );
        assert_eq!(three[2].top(), 152.0);
        assert!(
            gallery(&image(4), 404.0)
                .iter()
                .all(|rect| rect.right() <= 404.0)
        );
        assert_eq!(gallery(&image(2), 404.0)[1].height(), 220.0);
    }

    #[test]
    fn footers_join_text_and_a_medium_date() {
        assert_eq!(footer_line(Some("309K likes"), None), "309K likes");
        assert_eq!(footer_line(None, Some("not a date")), "");
        let line = footer_line(Some("309K likes"), Some("2006-03-21T20:50:14.000Z"));
        assert!(line.starts_with("309K likes · Mar 2"), "{line}");
        assert!(line.ends_with(", 2006"), "{line}");
    }
}
