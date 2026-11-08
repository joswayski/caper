//! What a message shows below its text: attachments (a gallery of images
//! and videos, then file cards), link preview cards and a KLIPY GIF. Built
//! when a row binds; pictures fill in as they load (`crate::media`).

use crate::images::{Icon, Images};
use crate::rows::{Action, Handler};
use caper_client::attachments::{frame_size, gallery, inline_source};
use caper_client::model::{Attachment, AttachmentKind, AttachmentStatus, Embed, EmbedBlock};
use gtk::{glib, pango, prelude::*};
use std::rc::Rc;

/// A gallery's width, as desktop and web cap it.
const GALLERY_WIDTH: f32 = 480.0;
/// Link preview cards and their media.
const CARD_WIDTH: i32 = 432;
const CARD_MEDIA: [f32; 2] = [400.0, 300.0];
const THUMBNAIL: i32 = 80;
/// KLIPY GIFs and stickers fit these squares.
const GIF_EDGE: f32 = 300.0;
const STICKER_EDGE: f32 = 160.0;

/// A message's content below its text, compared whole with its row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    /// Sent attachments' message/presentation origin; previews have no reveal state.
    pub scope: Option<String>,
    /// Each file with its freshest URLs, and whether it is a spoiler you
    /// have not revealed.
    pub attachments: Vec<(Attachment, bool)>,
    pub embeds: Vec<Embed>,
    pub gif: Option<caper_client::klipy::Gif>,
}

impl Content {
    pub fn is_empty(&self) -> bool {
        self.attachments.is_empty() && self.embeds.is_empty() && self.gif.is_none()
    }
}

fn media(attachment: &Attachment) -> bool {
    matches!(
        attachment.kind,
        AttachmentKind::Image | AttachmentKind::Video
    )
}

/// What the viewer can show: images and videos that are ready.
pub fn viewable(attachment: &Attachment) -> bool {
    media(attachment)
        && attachment.status == AttachmentStatus::Ready
        && !attachment.unavailable
        && attachment.url.is_some()
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    crate::ui::label(text, classes)
}

/// `m:ss`.
fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The view for `content`, or `None` when there is nothing to show.
pub fn view(images: &Rc<Images>, content: &Content, handler: &Handler) -> Option<gtk::Box> {
    if content.is_empty() {
        return None;
    }
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("content");
    root.set_halign(gtk::Align::Start);
    let tiles: Vec<&(Attachment, bool)> = content
        .attachments
        .iter()
        .filter(|(attachment, _)| media(attachment))
        .collect();
    // This message's images, for the viewer.
    let gallery_files: Vec<Attachment> = tiles
        .iter()
        .filter(|(attachment, hidden)| !hidden && viewable(attachment))
        .map(|(attachment, _)| attachment.clone())
        .collect();
    match tiles.as_slice() {
        [] => {}
        [(attachment, hidden)] => {
            let [width, height] = frame_size(attachment);
            let tile = tile(
                images,
                attachment,
                *hidden,
                (width as i32, height as i32),
                &gallery_files,
                handler,
                content.scope.as_deref(),
            );
            tile.add_css_class("single");
            root.append(&tile);
        }
        many => {
            let slots = gallery(many.len(), GALLERY_WIDTH);
            let fixed = gtk::Fixed::new();
            fixed.add_css_class("gallery");
            fixed.set_overflow(gtk::Overflow::Hidden);
            fixed.set_halign(gtk::Align::Start);
            for ((attachment, hidden), [x, y, width, height]) in many.iter().zip(slots) {
                let tile = tile(
                    images,
                    attachment,
                    *hidden,
                    (width.round() as i32, height.round() as i32),
                    &gallery_files,
                    handler,
                    content.scope.as_deref(),
                );
                fixed.put(&tile, x.into(), y.into());
            }
            root.append(&fixed);
        }
    }
    for (attachment, hidden) in content
        .attachments
        .iter()
        .filter(|(attachment, _)| !media(attachment))
    {
        if !attachment.spoiler && voice_playable(attachment) {
            root.append(&crate::playback::card(images, attachment));
        } else {
            root.append(&file_card(
                images,
                attachment,
                *hidden,
                handler,
                content.scope.as_deref(),
            ));
        }
    }
    for embed in &content.embeds {
        root.append(&card(images, embed, handler));
    }
    if let Some(gif) = &content.gif {
        root.append(&gif_view(images, gif, handler));
    }
    Some(root)
}

/// A ready voice message plays in place; a failed, processing or expired
/// one keeps its file card and notice.
fn voice_playable(attachment: &Attachment) -> bool {
    attachment.voice && tile_notice(attachment).is_none() && attachment.url.is_some()
}

/// A picture filling `size`, cropped to cover it.
fn picture(size: (i32, i32)) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_size_request(size.0, size.1);
    picture
}

/// A clickable frame with an accessible name.
fn button(name: &str, classes: &[&str]) -> gtk::Button {
    let button: gtk::Button = crate::app::named("GtkButton", name);
    button.add_css_class("flat");
    for class in classes {
        button.add_css_class(class);
    }
    button.set_halign(gtk::Align::Start);
    button.set_overflow(gtk::Overflow::Hidden);
    // A click must not leave the row's toolbar pinned open by focus.
    button.set_focus_on_click(false);
    button
}

fn centered(widget: &impl IsA<gtk::Widget>) {
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
    widget.set_can_target(false);
}

/// A notice drawn over a tile: removed, failed or still processing.
fn tile_notice(attachment: &Attachment) -> Option<&'static str> {
    if attachment.unavailable {
        Some("File removed")
    } else if attachment.status == AttachmentStatus::Failed {
        Some("Couldn't process this file")
    } else if attachment.status == AttachmentStatus::Processing {
        Some("Processing…")
    } else {
        None
    }
}

fn tile(
    images: &Rc<Images>,
    attachment: &Attachment,
    hidden: bool,
    size: (i32, i32),
    gallery_files: &[Attachment],
    handler: &Handler,
    scope: Option<&str>,
) -> gtk::Widget {
    let video = attachment.kind == AttachmentKind::Video;
    let overlay = gtk::Overlay::new();
    let shown = picture(size);
    overlay.set_child(Some(&shown));
    let name = if hidden {
        "Reveal spoiler attachment".to_owned()
    } else if video {
        format!("Play {}", attachment.name)
    } else {
        attachment.name.clone()
    };
    let tile = button(&name, &["tile"]);
    tile.set_size_request(size.0, size.1);
    tile.set_tooltip_text(Some(&attachment.name));
    if hidden {
        tile.add_css_class("spoiler");
        let pill = label("Spoiler", &["pill"]);
        centered(&pill);
        overlay.add_overlay(&pill);
        let (handler, id) = (handler.clone(), attachment.id.clone());
        if let Some(scope) = scope.map(str::to_owned) {
            tile.connect_clicked(move |_| {
                handler(Action::Reveal {
                    scope: scope.clone(),
                    id: id.clone(),
                    revealed: true,
                })
            });
        }
        tile.set_child(Some(&overlay));
        return tile.upcast();
    }
    if let Some(url) = inline_source(attachment).filter(|_| !attachment.unavailable) {
        images.media.bind(&shown, &attachment.id, url);
    }
    if let Some(notice) = tile_notice(attachment) {
        let text = label(notice, &["pill"]);
        centered(&text);
        overlay.add_overlay(&text);
    } else if video {
        let play = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        play.add_css_class("play");
        play.append(&images.icon_image(Icon::Play, "#f3f4f5", 20));
        centered(&play);
        overlay.add_overlay(&play);
        if attachment.animated {
            let badge = label("GIF", &["pill", "corner"]);
            badge.set_halign(gtk::Align::Start);
            badge.set_valign(gtk::Align::End);
            badge.set_can_target(false);
            overlay.add_overlay(&badge);
        } else if let Some(ms) = attachment.duration_ms {
            let badge = label(&duration(ms), &["pill", "corner"]);
            badge.set_halign(gtk::Align::End);
            badge.set_valign(gtk::Align::End);
            badge.set_can_target(false);
            overlay.add_overlay(&badge);
        }
    }
    tile.set_child(Some(&overlay));
    let ready = tile_notice(attachment).is_none() && attachment.url.is_some();
    tile.set_sensitive(ready);
    let view = {
        let (handler, attachment) = (handler.clone(), attachment.clone());
        let files = gallery_files.to_vec();
        let scope = scope.map(str::to_owned);
        move || match files.iter().position(|file| file.id == attachment.id) {
            Some(index) => handler(Action::View(files.clone(), index, scope.clone())),
            None => handler(Action::Open(attachment.clone())),
        }
    };
    if video && ready {
        return inline_video(images, attachment, &tile, size, view);
    }
    if ready {
        // Desktop's image menu: a right click, or the keyboard's menu key.
        let menu = {
            let (handler, attachment) = (handler.clone(), attachment.clone());
            move |anchor: &gtk::Widget| {
                handler(Action::ImageMenu {
                    attachment: attachment.clone(),
                    anchor: anchor.clone(),
                })
            }
        };
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_SECONDARY);
        let open = menu.clone();
        click.connect_pressed(move |gesture, _, _, _| {
            if let Some(tile) = gesture.widget() {
                open(&tile);
            }
        });
        tile.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |keys, key, _, modifiers| {
            let menu_key = key == gtk::gdk::Key::Menu
                || (key == gtk::gdk::Key::F10
                    && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK));
            match keys.widget() {
                Some(tile) if menu_key => {
                    menu(&tile);
                    gtk::glib::Propagation::Stop
                }
                _ => gtk::glib::Propagation::Proceed,
            }
        });
        tile.add_controller(keys);
    }
    tile.connect_clicked(move |_| view());
    tile.upcast()
}

/// A video tile that plays in place when clicked, with "Open in viewer"
/// over it, as on the desktop. GIF-style videos play by themselves (when
/// animations are on) while their tile is in view, and stop out of it.
fn inline_video(
    images: &Rc<Images>,
    attachment: &Attachment,
    tile: &gtk::Button,
    (width, height): (i32, i32),
    view: impl Fn() + 'static,
) -> gtk::Widget {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.add_css_class("inline-video");
    holder.set_size_request(width, height);
    holder.set_overflow(gtk::Overflow::Hidden);
    holder.append(tile);
    let expand: gtk::Button =
        crate::app::named("GtkButton", &format!("Open {} in viewer", attachment.name));
    let glyph = images.icon_image(Icon::ExternalLink, "#f3f4f5", 16);
    centered(&glyph);
    expand.set_child(Some(&glyph));
    expand.set_tooltip_text(Some("Open in viewer"));
    expand.add_css_class("video-expand");
    expand.set_halign(gtk::Align::End);
    expand.set_valign(gtk::Align::Start);
    // The holder keeps both: the poster tile, and the stage once playing.
    let playing = gtk::Overlay::new();
    playing.add_overlay(&expand);
    playing.set_visible(false);
    holder.append(&playing);
    // Back to the poster: the stage leaves the window and stops.
    let poster = {
        let (tile, playing) = (tile.downgrade(), playing.downgrade());
        move || {
            let (Some(tile), Some(playing)) = (tile.upgrade(), playing.upgrade()) else {
                return;
            };
            if playing.child().is_some() {
                playing.set_child(gtk::Widget::NONE);
                playing.set_visible(false);
                tile.set_visible(true);
            }
        }
    };
    let play = {
        let (images, attachment) = (Rc::downgrade(images), attachment.clone());
        let (tile, playing) = (tile.downgrade(), playing.downgrade());
        move || {
            let (Some(images), Some(tile), Some(playing)) =
                (images.upgrade(), tile.upgrade(), playing.upgrade())
            else {
                return;
            };
            if playing.child().is_none() {
                let video = crate::video::stage(&images, &attachment, width, height);
                playing.set_child(Some(&video.root));
                tile.set_visible(false);
                playing.set_visible(true);
            }
        }
    };
    let start = play.clone();
    tile.connect_clicked(move |_| start());
    let stop = poster.clone();
    expand.connect_clicked(move |_| {
        stop();
        view();
    });
    if attachment.animated {
        let holder_ref = holder.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
            let Some(holder) = holder_ref.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let animate =
                gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
            if animate && in_view(&holder) {
                play();
            } else {
                poster();
            }
            glib::ControlFlow::Continue
        });
    }
    holder.upcast()
}

/// Whether any of `widget` shows in its scrolled window's viewport.
fn in_view(widget: &impl IsA<gtk::Widget>) -> bool {
    let Some(scroller) = widget.ancestor(gtk::ScrolledWindow::static_type()) else {
        return widget.is_mapped();
    };
    widget.is_mapped()
        && widget.compute_bounds(&scroller).is_some_and(|bounds| {
            bounds.y() + bounds.height() > 0.0
                && bounds.y() < scroller.height() as f32
                && bounds.x() + bounds.width() > 0.0
                && bounds.x() < scroller.width() as f32
        })
}

fn file_card(
    images: &Images,
    attachment: &Attachment,
    hidden: bool,
    handler: &Handler,
    scope: Option<&str>,
) -> gtk::Button {
    let card = button(
        &if hidden {
            "Reveal spoiler attachment".to_owned()
        } else {
            format!("Open {}", attachment.name)
        },
        &["file-card"],
    );
    card.set_size_request(280, -1);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let failed = attachment.status == AttachmentStatus::Failed && !attachment.unavailable;
    line.append(&images.icon_image(
        if attachment.voice {
            Icon::AudioLines
        } else {
            Icon::FileText
        },
        if failed { "#ff9b82" } else { "#b9bcbe" },
        20,
    ));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let name = label(
        if hidden { "Spoiler" } else { &attachment.name },
        &["file-name"],
    );
    name.set_ellipsize(pango::EllipsizeMode::Middle);
    name.set_max_width_chars(28);
    text.append(&name);
    let size = caper_client::uploads::format_bytes(attachment.size);
    let detail = match tile_notice(attachment) {
        _ if hidden => "Click to reveal".to_owned(),
        Some(notice) => notice.to_owned(),
        None if attachment.voice => match attachment.duration_ms {
            Some(ms) => format!("Voice message · {}", duration(ms)),
            None => "Voice message".to_owned(),
        },
        None if attachment.kind == AttachmentKind::Audio => format!("Audio · {size}"),
        None => size,
    };
    text.append(&label(
        &detail,
        if failed {
            &["error", "small"]
        } else {
            &["muted", "small"]
        },
    ));
    if !hidden
        && let caper_client::model::TranscriptView::Text(transcript) = attachment.transcript_view()
    {
        let words = label(transcript, &["transcript"]);
        words.set_wrap(true);
        words.set_wrap_mode(pango::WrapMode::WordChar);
        words.set_max_width_chars(36);
        text.append(&words);
    }
    line.append(&text);
    if !hidden {
        line.append(&images.icon_image(Icon::Download, "#b9bcbe", 16));
    }
    card.set_child(Some(&line));
    let handler = handler.clone();
    let attachment = attachment.clone();
    if hidden {
        if let Some(scope) = scope.map(str::to_owned) {
            card.connect_clicked(move |_| {
                handler(Action::Reveal {
                    scope: scope.clone(),
                    id: attachment.id.clone(),
                    revealed: true,
                })
            });
        }
    } else {
        card.set_sensitive(tile_notice(&attachment).is_none() && attachment.url.is_some());
        card.connect_clicked(move |_| handler(Action::Open(attachment.clone())));
    }
    card
}

/// `[width, height]` scaled to fit `bounds` without enlarging; a missing
/// size takes the bounds' shape.
fn fit(width: Option<u32>, height: Option<u32>, bounds: [f32; 2]) -> (i32, i32) {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (bounds[0] / width).min(bounds[1] / height).min(1.0);
            (
                (width * scale).round().max(1.0) as i32,
                (height * scale).round().max(1.0) as i32,
            )
        }
        _ => (bounds[0] as i32, (bounds[0] * 9.0 / 16.0) as i32),
    }
}

fn link_markup(text: &str, url: Option<&str>) -> String {
    let text = glib::markup_escape_text(text);
    match url.filter(|url| url.starts_with("https://") || url.starts_with("http://")) {
        Some(url) => format!("<a href=\"{}\">{text}</a>", glib::markup_escape_text(url)),
        None => text.to_string(),
    }
}

fn wrapped(markup: &str, classes: &[&str]) -> gtk::Label {
    let label = label("", classes);
    label.set_markup(markup);
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label.set_max_width_chars(52);
    label.set_lines(8);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.connect_activate_link(|_, uri| {
        crate::media::open_uri(uri);
        glib::Propagation::Stop
    });
    label
}

/// A link preview card: site, author, title, text, media and footer.
fn card(images: &Images, embed: &Embed, handler: &Handler) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("embed");
    card.set_size_request(CARD_WIDTH, -1);
    card.set_halign(gtk::Align::Start);
    let heading = caper_client::embeds::heading(embed).to_owned();
    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    text.set_hexpand(true);
    text.append(&wrapped(
        &glib::markup_escape_text(&embed.site.name),
        &["muted", "small"],
    ));
    let mut thumbnail = None;
    let mut media = Vec::new();
    let mut footer = None;
    for block in &embed.blocks {
        match block {
            EmbedBlock::Author {
                name, handle, url, ..
            } => {
                let name = match handle {
                    Some(handle) => format!("{name} {handle}"),
                    None => name.clone(),
                };
                text.append(&wrapped(
                    &link_markup(&name, url.as_deref()),
                    &["small", "author"],
                ));
            }
            EmbedBlock::Title { text: title, url } => {
                text.append(&wrapped(
                    &link_markup(title, url.as_deref().or(Some(&embed.url))),
                    &["embed-title"],
                ));
            }
            EmbedBlock::Text { text: body } => {
                text.append(&wrapped(&glib::markup_escape_text(body), &["embed-text"]));
            }
            EmbedBlock::Images {
                items,
                thumbnail: true,
            } if items.len() == 1 => thumbnail = Some(items[0].clone()),
            EmbedBlock::Images { items, .. } => {
                media.push(images_view(images, items, &heading, handler))
            }
            EmbedBlock::Video {
                url,
                poster,
                width,
                height,
                ..
            } => media.push(poster_view(
                images,
                poster.as_deref(),
                fit(*width, *height, CARD_MEDIA),
                &format!("Play {heading}"),
                url,
                handler,
            )),
            EmbedBlock::Player {
                poster,
                width,
                height,
                ..
            } => media.push(poster_view(
                images,
                poster.as_deref(),
                fit(*width, *height, CARD_MEDIA),
                &format!("Play {heading}"),
                &embed.url,
                handler,
            )),
            EmbedBlock::Footer { text, timestamp } => {
                let line = caper_client::embeds::footer_line(text.as_deref(), timestamp.as_deref());
                if !line.is_empty() {
                    footer = Some(line);
                }
            }
        }
    }
    match thumbnail {
        Some(image) => {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.append(&text);
            let files = caper_client::embeds::image_files(std::slice::from_ref(&image), &heading);
            let tile = button(&format!("View {heading}"), &["tile", "thumbnail"]);
            tile.set_valign(gtk::Align::Start);
            let shown = picture((THUMBNAIL, THUMBNAIL));
            images
                .media
                .bind(&shown, &caper_client::embeds::key(&image.url), &image.url);
            tile.set_child(Some(&shown));
            let handler = handler.clone();
            tile.connect_clicked(move |_| handler(Action::View(files.clone(), 0, None)));
            line.append(&tile);
            card.append(&line);
        }
        None => card.append(&text),
    }
    for view in media {
        card.append(&view);
    }
    if let Some(footer) = footer {
        card.append(&label(&footer, &["muted", "small"]));
    }
    card
}

fn images_view(
    images: &Images,
    items: &[caper_client::model::EmbedImage],
    heading: &str,
    handler: &Handler,
) -> gtk::Widget {
    let files = caper_client::embeds::image_files(items, heading);
    let place = |index: usize, size: (i32, i32)| {
        let item = &items[index];
        let tile = button(&format!("View {heading}"), &["tile"]);
        tile.set_size_request(size.0, size.1);
        let shown = picture(size);
        images
            .media
            .bind(&shown, &caper_client::embeds::key(&item.url), &item.url);
        tile.set_child(Some(&shown));
        let (handler, files) = (handler.clone(), files.clone());
        tile.connect_clicked(move |_| handler(Action::View(files.clone(), index, None)));
        tile
    };
    if items.len() == 1 {
        let tile = place(0, fit(items[0].width, items[0].height, CARD_MEDIA));
        tile.add_css_class("single");
        return tile.upcast();
    }
    let fixed = gtk::Fixed::new();
    fixed.add_css_class("gallery");
    fixed.set_overflow(gtk::Overflow::Hidden);
    fixed.set_halign(gtk::Align::Start);
    for (index, [x, y, width, height]) in
        gallery(items.len(), CARD_MEDIA[0]).into_iter().enumerate()
    {
        let tile = place(index, (width.round() as i32, height.round() as i32));
        fixed.put(&tile, x.into(), y.into());
    }
    fixed.upcast()
}

/// A video or player's poster with a play button that opens `url`.
fn poster_view(
    images: &Images,
    poster: Option<&str>,
    size: (i32, i32),
    name: &str,
    url: &str,
    handler: &Handler,
) -> gtk::Widget {
    let tile = button(name, &["tile", "single"]);
    tile.set_size_request(size.0, size.1);
    let overlay = gtk::Overlay::new();
    let shown = picture(size);
    if let Some(poster) = poster {
        images
            .media
            .bind(&shown, &caper_client::embeds::key(poster), poster);
    }
    overlay.set_child(Some(&shown));
    let play = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    play.add_css_class("play");
    play.append(&images.icon_image(Icon::Play, "#f3f4f5", 20));
    centered(&play);
    overlay.add_overlay(&play);
    tile.set_child(Some(&overlay));
    let (handler, url) = (handler.clone(), url.to_owned());
    tile.connect_clicked(move |_| handler(Action::OpenUrl(url.clone())));
    tile.upcast()
}

/// A sent KLIPY GIF or sticker, animated when the format is.
fn gif_view(images: &Images, gif: &caper_client::klipy::Gif, handler: &Handler) -> gtk::Button {
    let edge = match gif.kind {
        caper_client::klipy::Kind::Gif => GIF_EDGE,
        caper_client::klipy::Kind::Sticker => STICKER_EDGE,
    };
    let size = fit(Some(gif.width), Some(gif.height), [edge, edge]);
    let tile = button(gif.alt(), &["tile", "gif"]);
    if gif.kind == caper_client::klipy::Kind::Sticker {
        tile.add_css_class("sticker");
    }
    tile.set_size_request(size.0, size.1);
    let shown = picture(size);
    shown.set_content_fit(gtk::ContentFit::Contain);
    if let Some(url) = gif
        .gif
        .as_ref()
        .or(gif.webp.as_ref())
        .or(gif.still.as_ref())
    {
        images
            .media
            .bind(&shown, &caper_client::klipy::media_key(url), url);
    }
    tile.set_child(Some(&shown));
    let (handler, file) = (handler.clone(), caper_client::klipy::viewer_file(gif));
    tile.connect_clicked(move |_| handler(Action::View(vec![file.clone()], 0, None)));
    tile
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_minutes_and_seconds() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(65_400), "1:05");
        assert_eq!(duration(600_000), "10:00");
    }

    #[test]
    fn media_fits_without_enlarging_and_keeps_its_shape() {
        assert_eq!(fit(Some(1600), Some(900), CARD_MEDIA), (400, 225));
        assert_eq!(fit(Some(900), Some(1600), CARD_MEDIA), (169, 300));
        assert_eq!(fit(Some(120), Some(80), CARD_MEDIA), (120, 80));
        assert_eq!(fit(None, Some(80), CARD_MEDIA), (400, 225));
    }
}
