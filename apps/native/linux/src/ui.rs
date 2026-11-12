//! The window's widgets, built once. `app` fills and updates them.

use crate::images::{AvatarView, Icon, Images};
use crate::rows;
use gtk::{gio, glib, pango, prelude::*};
use std::rc::Rc;

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// Gives every toplevel, now and later, the hand cursor over its controls
/// (`hands`); the windows that open later (dialogs, the viewer) also
/// `arrive` whenever they show.
pub fn watch_toplevels() {
    let toplevels = gtk::Window::toplevels();
    for window in toplevels.iter::<gtk::Widget>().flatten() {
        hands(&window);
    }
    toplevels.connect_items_changed(move |model, position, _, added| {
        for position in position..position + added {
            if let Some(window) = model.item(position).and_downcast::<gtk::Widget>() {
                hands(&window);
                arrive(&window);
            }
        }
    });
}

/// Plays `widget`'s CSS entrance (style.css's `.arriving` rules) each time
/// it shows, as desktop's `motion`: things arrive rather than appear.
/// Hiding stays instant, and GTK skips the animation when animations are
/// off (reduced motion).
pub fn arrive(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.as_ref();
    if widget.has_css_class("arrives") {
        return;
    }
    widget.add_css_class("arrives");
    widget.connect_map(|widget| widget.add_css_class("arriving"));
    widget.connect_unmap(|widget| widget.remove_css_class("arriving"));
}

/// As desktop, a hand over anything clickable inside `native` (a window or
/// popover): buttons, switches, check boxes, rail tiles, channel rows and
/// single-click grid cells. GTK has no CSS cursor, so the control under the
/// pointer gets one the first time it is hovered. Widgets with their own
/// cursor (text, links, resize handles) keep it, and a disabled control is
/// never picked, so it keeps the arrow.
pub fn hands(native: &impl IsA<gtk::Widget>) {
    let native = native.as_ref();
    let named = |controller: glib::Object| {
        controller
            .downcast::<gtk::EventController>()
            .is_ok_and(|controller| controller.name().as_deref() == Some("hands"))
    };
    if native
        .observe_controllers()
        .iter::<glib::Object>()
        .flatten()
        .any(named)
    {
        return;
    }
    let motion = gtk::EventControllerMotion::new();
    motion.set_name(Some("hands"));
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let point = |motion: &gtk::EventControllerMotion, x: f64, y: f64| {
        let Some(native) = motion.widget() else {
            return;
        };
        let Some(target) = native.pick(x, y, gtk::PickFlags::DEFAULT) else {
            return;
        };
        if let Some(widget) = std::iter::successors(Some(target), gtk::Widget::parent)
            .take_while(|widget| widget.cursor().is_none())
            .find(clickable)
        {
            widget.set_cursor_from_name(Some("pointer"));
        }
    };
    motion.connect_enter(point);
    motion.connect_motion(point);
    native.add_controller(motion);
}

fn clickable(widget: &gtk::Widget) -> bool {
    let single_click = |parent: Option<gtk::Widget>| match parent {
        Some(parent) => {
            parent
                .downcast_ref::<gtk::GridView>()
                .is_some_and(gtk::GridView::is_single_click_activate)
                || parent
                    .downcast_ref::<gtk::ListView>()
                    .is_some_and(gtk::ListView::is_single_click_activate)
        }
        None => false,
    };
    widget.is::<gtk::Button>()
        || widget.is::<gtk::CheckButton>()
        || widget.is::<gtk::Switch>()
        || widget
            .downcast_ref::<gtk::ListBoxRow>()
            .is_some_and(ListBoxRowExt::is_activatable)
        || widget
            .downcast_ref::<gtk::FlowBoxChild>()
            .is_some_and(|child| {
                child
                    .parent()
                    .and_downcast::<gtk::FlowBox>()
                    .is_some_and(|flow| flow.activates_on_single_click())
            })
        || single_click(widget.parent())
}

/// The composer's lip: a non-interactive shade over the bottom of a message
/// history, where it meets the composer. It takes no layout space.
fn composer_lip() -> gtk::Box {
    let lip = gtk::Box::new(gtk::Orientation::Vertical, 0);
    lip.add_css_class("composer-lip");
    lip.set_valign(gtk::Align::End);
    lip.set_can_target(false);
    lip
}

/// A message field: a `GtkTextView` with a placeholder. As desktop and web on
/// a keyboard, Enter sends; the channel's composer has no Send button. A
/// thread's composer, as desktop's, puts "Also send to" on its own row and
/// Send reply beside the field.
pub struct Composer {
    /// What the layout holds: the field, and a thread's rows around it.
    pub outer: gtk::Box,
    pub root: gtk::Box,
    /// The files staged in it (`crate::uploads`).
    pub chips: gtk::Box,
    pub attach: gtk::Button,
    pub view: gtk::TextView,
    /// Its formatting: hidden delimiters, styled runs and typing as web's.
    pub rich: Rc<crate::composing::Rich>,
    pub placeholder: gtk::Label,
    /// "Send a GIF", shown while GIFs are on.
    pub gif: gtk::Button,
    /// A thread reply's "Also send to #channel" (hidden in the channel's
    /// composer).
    pub broadcast: gtk::CheckButton,
    /// A thread's Send reply, beside the field.
    pub send: Option<gtk::Button>,
    /// Where a narrow thread's microphone goes: beside "Also send to"
    /// (`app::panes`).
    pub mic_slot: Option<gtk::Box>,
    /// The draft's link preview card, with "Remove link preview" (main
    /// composer only).
    pub preview: gtk::Box,
    /// The input row, which the recording bar replaces while recording.
    pub line: gtk::Box,
    /// The microphone: it dictates (`app::recording`).
    pub mic: gtk::Button,
    /// Insert emoji, as on desktop.
    pub emoji: gtk::Button,
    pub recording: RecordingBar,
    /// Transcribing (Cancel), or failed (Retry, Keep as voice note,
    /// Discard), above the input.
    pub dictation: DictationRow,
    /// Voice messages sending on their own: progress with Cancel, or
    /// \"not sent\" with Retry and Discard (`app::recording`).
    pub sends: gtk::Box,
}

/// `[trash] ● 0:07 ▁▃▅▇▅ [Voice note] [done]` in the input row's place.
pub struct RecordingBar {
    pub root: gtk::Box,
    pub discard: gtk::Button,
    pub dot: gtk::Box,
    pub timer: gtk::Label,
    /// The live input meter (`app::recording` draws it).
    pub meter: gtk::DrawingArea,
    /// "Starting microphone…", "Transcribing…" or "Adding to your
    /// message…" over the meter.
    pub note: gtk::Label,
    pub voice_note: gtk::Button,
    pub done: gtk::Button,
}

pub struct DictationRow {
    pub root: gtk::Box,
    pub spinner: gtk::Spinner,
    pub text: gtk::Label,
    pub retry: gtk::Button,
    pub keep: gtk::Button,
    /// Cancel while transcribing, Discard once it failed.
    pub drop: gtk::Button,
}

impl RecordingBar {
    fn new(images: &Images) -> Self {
        // Named for the recording; the timer's text reads its length.
        let root: gtk::Box = crate::app::named("GtkBox", "Recording voice message");
        root.set_orientation(gtk::Orientation::Horizontal);
        root.set_spacing(8);
        root.add_css_class("recording-bar");
        root.set_visible(false);
        let discard: gtk::Button = crate::app::named("GtkButton", "Discard dictation");
        discard.set_child(Some(&images.icon_image(Icon::Trash, "#b9bcbe", 18)));
        discard.set_tooltip_text(Some("Discard dictation"));
        discard.add_css_class("flat");
        discard.set_valign(gtk::Align::Center);
        root.append(&discard);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("recording-dot");
        dot.set_valign(gtk::Align::Center);
        dot.set_halign(gtk::Align::Center);
        root.append(&dot);
        let timer = label("0:00", &["recording-timer"]);
        root.append(&timer);
        let meter = gtk::DrawingArea::new();
        meter.set_hexpand(true);
        let note = label("", &["muted", "small"]);
        note.set_xalign(0.0);
        note.set_valign(gtk::Align::Center);
        let stage = gtk::Overlay::new();
        stage.set_child(Some(&meter));
        stage.add_overlay(&note);
        stage.set_hexpand(true);
        root.append(&stage);
        let voice_note: gtk::Button = crate::app::named("GtkButton", "Voice note");
        voice_note.set_label("Voice note");
        voice_note.add_css_class("voice-note");
        voice_note.set_valign(gtk::Align::Center);
        root.append(&voice_note);
        let done: gtk::Button = crate::app::named("GtkButton", "Done, insert text");
        done.set_child(Some(&images.icon_image(Icon::Check, "#f3f4f5", 16)));
        done.set_tooltip_text(Some("Done, insert text"));
        done.add_css_class("recording-done");
        done.set_valign(gtk::Align::Center);
        root.append(&done);
        Self {
            root,
            discard,
            dot,
            timer,
            meter,
            note,
            voice_note,
            done,
        }
    }
}

impl DictationRow {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        root.add_css_class("dictation-row");
        root.set_visible(false);
        let spinner = gtk::Spinner::new();
        spinner.set_size_request(12, 12);
        root.append(&spinner);
        let text = label("", &["small"]);
        text.set_wrap(true);
        text.set_xalign(0.0);
        root.append(&text);
        let small = |name: &str| {
            let button = gtk::Button::with_label(name);
            button.add_css_class("small-button");
            button.set_valign(gtk::Align::Center);
            root.append(&button);
            button
        };
        let retry = small("Retry");
        let keep = small("Keep as voice note");
        let drop = small("Cancel");
        Self {
            root,
            spinner,
            text,
            retry,
            keep,
            drop,
        }
    }
}

impl Composer {
    fn new(images: &Images, thread: bool) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("composer");
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        chips.add_css_class("chips");
        chips.set_visible(false);
        let chip_scroller = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&chips)
            .build();
        // Hidden with its chips.
        chips
            .bind_property("visible", &chip_scroller, "visible")
            .sync_create()
            .build();
        root.append(&chip_scroller);
        let preview = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        preview.add_css_class("composer-preview");
        preview.set_halign(gtk::Align::Start);
        preview.set_visible(false);
        root.append(&preview);
        let dictation = DictationRow::new();
        root.append(&dictation.root);
        let sends = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sends.set_visible(false);
        root.append(&sends);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.append(&line);
        let recording = RecordingBar::new(images);
        root.append(&recording.root);
        let attach: gtk::Button = crate::app::named("GtkButton", "Attach files");
        attach.set_child(Some(&images.icon_image(Icon::Plus, "#b9bcbe", 18)));
        attach.set_tooltip_text(Some("Attach files"));
        attach.set_valign(gtk::Align::End);
        attach.add_css_class("flat");
        attach.add_css_class("composer-tool");
        line.append(&attach);
        let view: gtk::TextView = crate::app::named("GtkTextView", "Message");
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_accepts_tab(false);
        view.set_hexpand(true);
        let rich = crate::composing::attach(&view, &root);
        let placeholder = label("", &["placeholder"]);
        placeholder.set_can_target(false);
        placeholder.set_valign(gtk::Align::Start);
        placeholder.set_ellipsize(pango::EllipsizeMode::End);
        let field = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(200)
            .child(&view)
            .hexpand(true)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&field));
        overlay.add_overlay(&placeholder);
        overlay.set_hexpand(true);
        line.append(&overlay);
        let gif: gtk::Button = crate::app::named("GtkButton", "Send a GIF");
        let badge = gtk::Label::new(Some("GIF"));
        badge.add_css_class("gif-badge");
        badge.set_halign(gtk::Align::Center);
        badge.set_valign(gtk::Align::Center);
        gif.set_child(Some(&badge));
        gif.set_tooltip_text(Some("Send a GIF"));
        gif.set_valign(gtk::Align::End);
        gif.add_css_class("flat");
        gif.add_css_class("composer-tool");
        gif.set_visible(false);
        line.append(&gif);
        let emoji: gtk::Button = crate::app::named("GtkButton", "Insert emoji");
        emoji.set_child(Some(&images.icon_image(Icon::Smile, "#b9bcbe", 18)));
        emoji.set_tooltip_text(Some("Insert emoji"));
        emoji.set_valign(gtk::Align::End);
        emoji.add_css_class("flat");
        emoji.add_css_class("composer-tool");
        line.append(&emoji);
        let mic: gtk::Button = crate::app::named("GtkButton", "Dictate");
        mic.set_child(Some(&images.icon_image(Icon::Mic, "#b9bcbe", 18)));
        mic.set_tooltip_text(Some("Dictate"));
        mic.set_valign(gtk::Align::End);
        mic.add_css_class("flat");
        mic.add_css_class("composer-tool");
        line.append(&mic);
        let empty = placeholder.clone();
        view.buffer()
            .connect_changed(move |buffer| empty.set_visible(buffer.char_count() == 0));
        let broadcast: gtk::CheckButton =
            crate::app::named("GtkCheckButton", "Also send to channel");
        broadcast.add_css_class("broadcast");
        // "Also send to" and the destination in bold (`app` names it).
        broadcast.set_child(Some(&gtk::Label::new(None)));
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let (mut send, mut mic_slot) = (None, None);
        if thread {
            outer.add_css_class("thread-composer");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("broadcast-row");
            broadcast.set_hexpand(true);
            row.append(&broadcast);
            let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.append(&slot);
            outer.append(&row);
            let input = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            root.set_hexpand(true);
            input.append(&root);
            let button: gtk::Button = crate::app::named("GtkButton", "Send reply");
            // Dark on eggshell once there is something to send, else muted.
            let icon = |color| {
                let image = images.icon_image(Icon::Send, color, 16);
                image.set_halign(gtk::Align::Center);
                image.set_valign(gtk::Align::Center);
                image
            };
            let (dark, dim) = (icon("#0c0d0f"), icon("#b9bcbe"));
            button.set_child(Some(&dim));
            button.set_tooltip_text(Some("Send reply"));
            button.add_css_class("send-reply");
            button.set_valign(gtk::Align::End);
            input.append(&button);
            outer.append(&input);
            let ready = {
                let (button, field, shown) = (button.clone(), view.clone(), chips.clone());
                move || {
                    let some = field.buffer().char_count() > 0 || shown.is_visible();
                    if some == button.has_css_class("ready") {
                        return;
                    }
                    if some {
                        button.add_css_class("ready");
                        button.set_child(Some(&dark));
                    } else {
                        button.remove_css_class("ready");
                        button.set_child(Some(&dim));
                    }
                }
            };
            let changed = ready.clone();
            view.buffer().connect_changed(move |_| changed());
            chips.connect_visible_notify(move |_| ready());
            send = Some(button);
            mic_slot = Some(slot);
        } else {
            outer.append(&root);
        }
        Self {
            outer,
            send,
            mic_slot,
            root,
            chips,
            attach,
            view,
            rich,
            placeholder,
            gif,
            broadcast,
            preview,
            line,
            mic,
            emoji,
            recording,
            dictation,
            sends,
        }
    }

    /// The draft's source, hidden delimiters included.
    pub fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }
}

/// A message list: the store, the virtualized view and its scroller.
pub struct List {
    pub store: gio::ListStore,
    pub view: gtk::ListView,
    pub scroller: gtk::ScrolledWindow,
}

impl List {
    /// `spare` rows are built ahead for pages of history (`rows::factory`).
    fn new(name: &str, images: &Rc<Images>, handler: rows::Handler, spare: usize) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let view: gtk::ListView = crate::app::named("GtkListView", name);
        view.set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        view.set_factory(Some(&rows::factory(images.clone(), handler, spare)));
        view.add_css_class("messages");
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&view)
            .build();
        Self {
            store,
            view,
            scroller,
        }
    }
}

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    /// "Zoom N%", shown briefly at the top right when the zoom changes.
    pub zoom_notice: gtk::Label,
    pub pages: gtk::Stack,
    pub steps: gtk::Stack,
    pub signin_actions: gtk::Stack,
    pub email: gtk::Entry,
    pub email_button: LoginAction,
    pub code: gtk::Entry,
    pub code_button: LoginAction,
    pub last_attempt: gtk::Label,
    pub spam_hint: gtk::Label,
    pub resend: gtk::Button,
    pub other_email: gtk::Button,
    pub code_hint: gtk::Label,
    pub signin_status: gtk::Label,
    pub signin_error: gtk::Label,
    /// The first-space page: a signed-in account with no spaces names one.
    pub first_name: gtk::Entry,
    pub first_create: gtk::Button,
    pub first_limit: gtk::Label,
    pub first_error: gtk::Label,
    pub first_logout: gtk::Button,
    /// "Notifications" on the first-space page.
    pub first_inbox: gtk::Button,
    pub rail: gtk::Box,
    pub sidebar: gtk::Box,
    /// Narrow windows: Browse in the conversation header, Close navigation
    /// in Browse, and the member list's drawer (`app::narrow`).
    pub browse: gtk::Button,
    pub browse_close: gtk::Button,
    pub members_drawer: gtk::Revealer,
    pub members_backdrop: gtk::Box,
    pub space_name: gtk::Label,
    pub space_header: gtk::Button,
    pub new_channel: gtk::Button,
    /// "Toggle channels": collapsed, only channels with live voice show.
    pub channels_toggle: gtk::Button,
    pub channels_count: gtk::Label,
    pub channel_invites: gtk::Box,
    pub channels: gtk::ListBox,
    /// "Message requests" (its first row toggles) and the incoming requests.
    pub requests: gtk::ListBox,
    /// "Notifications" or "Notifications · N": opens the inbox.
    pub inbox: gtk::Button,
    pub directs: gtk::ListBox,
    pub account_avatar: AvatarView,
    pub account_name: gtk::Label,
    pub settings: gtk::Button,
    pub new_direct: gtk::Button,
    /// The call card above the account bar, and the bar's voice toggles.
    pub call_card: gtk::Box,
    /// "Update available" above the account bar.
    pub update_dock: gtk::Button,
    pub mic: gtk::Button,
    pub deafen: gtk::Button,
    pub input_options: gtk::Button,
    pub output_options: gtk::Button,
    /// The header's Join voice, Switch here, Joining… or Leave voice.
    pub voice_join: gtk::Button,
    pub visit_back: gtk::Button,
    pub feedback: gtk::ListBox,
    pub feedback_waiting: gtk::Label,
    pub feedback_page: gtk::Box,
    /// A DM's person, before its name (as desktop).
    pub channel_avatar: AvatarView,
    pub channel_title: gtk::Label,
    pub status: gtk::Label,
    pub pins_button: gtk::Button,
    pub members_toggle: gtk::ToggleButton,
    pub members_panel: gtk::Box,
    /// Sidebar | the rest, and conversation | members (`app::panes`).
    pub split: gtk::Paned,
    /// Channels above, Direct messages below.
    pub sidebar_lists: gtk::Paned,
    pub content_split: gtk::Paned,
    pub members_title: gtk::Label,
    pub members_count: gtk::Label,
    pub member_search_button: gtk::Button,
    pub members_close: gtk::Button,
    pub member_search: gtk::SearchEntry,
    pub members: gtk::ListBox,
    pub list: List,
    pub error: gtk::Label,
    /// The banner's way to try again: Retry, Try again or Retry session.
    pub error_retry: gtk::Button,
    /// Over an empty conversation: "No messages yet." and its note, or
    /// "No joined channels" with Create channel for owners.
    pub empty: gtk::Box,
    pub empty_icon: gtk::Image,
    pub empty_title: gtk::Label,
    pub empty_note: gtk::Label,
    pub empty_action: gtk::Button,
    pub compose_bar: gtk::Box,
    pub compose_label: gtk::Label,
    pub compose_cancel: gtk::Button,
    /// After an edit failed (another device changed the message): its
    /// latest text replaces the draft, as desktop's editor.
    pub compose_reload: gtk::Button,
    pub composer: Composer,
    pub composer_error: gtk::Label,
    pub typing: gtk::Label,
    /// Away from the live end: "Load newer messages" and "Back to latest
    /// messages".
    pub newer_bar: gtk::Box,
    pub load_newer: gtk::Button,
    pub latest: gtk::Button,
    pub join_bar: gtk::Box,
    pub join_label: gtk::Label,
    pub join: gtk::Button,
    /// In place of the composer: an incoming request's Accept, Decline and
    /// Block, or Unblock for someone you blocked.
    pub request: RequestBar,
    pub thread_panel: gtk::Box,
    /// The conversation and the thread beside it.
    pub thread_split: gtk::Paned,
    /// The conversation (and the narrow member drawer over it).
    pub stage: gtk::Overlay,
    /// "in #channel" or "with Name", under "Thread".
    pub thread_place: gtk::Label,
    pub conversation: gtk::Box,
    pub drop: gtk::Box,
    pub drop_title: gtk::Label,
    pub drop_detail: gtk::Label,
    pub thread_close: gtk::Button,
    /// A narrow thread's way back, as desktop: "Back to channel", or "Back
    /// to conversation" in a DM (GTK 4.8 can't rename one button).
    pub thread_back: [gtk::Button; 2],
    pub thread_list: List,
    pub thread_composer: Composer,
    pub thread_error: gtk::Label,
}

impl Ui {
    pub fn build(
        application: &gtk::Application,
        images: &Rc<Images>,
        main_rows: rows::Handler,
        thread_rows: rows::Handler,
    ) -> Self {
        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Caper")
            .default_width(1440)
            .default_height(900)
            .build();
        window.add_css_class("caper");
        let pages = gtk::Stack::new();

        let loading = label("Connecting to Caper…", &["muted"]);
        loading.set_halign(gtk::Align::Center);
        pages.add_named(&loading, Some("loading"));

        // Sign-in, as desktop: the wordmark over a 440px column (email,
        // then the emailed code), its action right-aligned under the field.
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("signin-column");
        column.set_halign(gtk::Align::Center);
        column.set_size_request(440, -1);
        // GTK 4.8 doesn't name a picture after its alternative text.
        let signin_wordmark: gtk::Picture = crate::app::named("GtkPicture", "Caper");
        if let Some(texture) = crate::images::wordmark(132, 35) {
            signin_wordmark.set_paintable(Some(&texture));
        }
        signin_wordmark.set_size_request(132, 35);
        signin_wordmark.set_can_shrink(true);
        signin_wordmark.set_content_fit(gtk::ContentFit::ScaleDown);
        signin_wordmark.set_alternative_text(Some("Caper"));
        let signin_wordmark_line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        signin_wordmark_line.add_css_class("wordmark");
        signin_wordmark_line.set_halign(gtk::Align::Start);
        signin_wordmark_line.append(&signin_wordmark);
        column.append(&signin_wordmark_line);
        let steps = gtk::Stack::new();
        steps.set_vhomogeneous(false);
        let step = |title: &str, intro: &gtk::Label, field: &str, entry: &gtk::Entry| {
            let step = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let title = label(title, &["first-title"]);
            title.set_xalign(0.0);
            step.append(&title);
            intro.add_css_class("signin-intro");
            intro.set_wrap(true);
            intro.set_xalign(0.0);
            // Wrapped labels ask for their whole text's width; keep 440px.
            intro.set_max_width_chars(44);
            step.append(intro);
            let field = label(field, &["field", "signin-field"]);
            field.set_xalign(0.0);
            // The label names the field for assistive technologies.
            field.set_mnemonic_widget(Some(entry));
            step.append(&field);
            entry.add_css_class("signin-entry");
            step.append(entry);
            step
        };
        let email = gtk::Entry::builder()
            .placeholder_text("you@example.com")
            .input_purpose(gtk::InputPurpose::Email)
            .build();
        let intro = label(
            "Use your email to create an account or log back in.",
            &["muted"],
        );
        steps.add_named(
            &step("Welcome to Caper", &intro, "Email address", &email),
            Some("email"),
        );
        let code = gtk::Entry::builder()
            .placeholder_text("e.g. ABC234")
            .build();
        let code_hint = label("", &["muted"]);
        steps.add_named(
            &step("Check your email.", &code_hint, "Sign-in code", &code),
            Some("code"),
        );
        column.append(&steps);
        let signin_error = label("", &["error", "signin-error"]);
        signin_error.set_wrap(true);
        signin_error.set_xalign(0.0);
        column.append(&signin_error);
        let signin_status = label("", &["muted", "signin-status"]);
        signin_status.set_wrap(true);
        signin_status.set_xalign(0.0);
        column.append(&signin_status);
        let last_attempt = label(
            "One attempt left. Check the code carefully.",
            &["field", "signin-status"],
        );
        last_attempt.set_wrap(true);
        last_attempt.set_xalign(0.0);
        column.append(&last_attempt);
        let actions = gtk::Stack::new();
        actions.set_vhomogeneous(false);
        let email_button = LoginAction::new(images, "Email me a code", false);
        actions.add_named(&email_button.button, Some("email"));
        let code_actions = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let code_button = LoginAction::new(images, "Continue", true);
        code_actions.append(&code_button.button);
        let links = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        links.add_css_class("signin-links");
        let other_email = gtk::Button::with_label("Use a different email");
        other_email.add_css_class("flat");
        let resend = gtk::Button::with_label("Resend code");
        resend.add_css_class("flat");
        links.append(&other_email);
        links.append(&resend);
        code_actions.append(&links);
        let spam_hint = label(
            "Still nothing? Check your spam folder, or try again in 15 minutes.",
            &["muted", "signin-spam"],
        );
        spam_hint.set_wrap(true);
        spam_hint.set_xalign(0.0);
        code_actions.append(&spam_hint);
        actions.add_named(&code_actions, Some("code"));
        column.append(&actions);
        let signin = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&column)
            .build();
        signin.add_css_class("signin");
        pages.add_named(&signin, Some("signin"));

        // Web's first-space page.
        let first = gtk::Box::new(gtk::Orientation::Vertical, 0);
        first.add_css_class("first-space");
        first.set_halign(gtk::Align::Center);
        first.set_size_request(440, -1);
        // GTK 4.8 doesn't name a picture after its alternative text.
        let wordmark: gtk::Picture = crate::app::named("GtkPicture", "Caper");
        if let Some(texture) = crate::images::wordmark(132, 35) {
            wordmark.set_paintable(Some(&texture));
        }
        wordmark.set_size_request(132, 35);
        wordmark.set_can_shrink(true);
        wordmark.set_content_fit(gtk::ContentFit::ScaleDown);
        wordmark.set_halign(gtk::Align::Start);
        wordmark.set_alternative_text(Some("Caper"));
        let wordmark_line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        wordmark_line.add_css_class("wordmark");
        wordmark_line.set_halign(gtk::Align::Start);
        wordmark_line.append(&wordmark);
        first.append(&wordmark_line);
        let first_inbox = gtk::Button::with_label("Notifications");
        first_inbox.add_css_class("inbox-button");
        first_inbox.set_halign(gtk::Align::Start);
        first.append(&first_inbox);
        let first_title = label("Name your space", &["first-title"]);
        first_title.set_xalign(0.0);
        first.append(&first_title);
        let first_intro = label(
            "Choose something you will recognize easily. You can always change it later!",
            &["first-intro"],
        );
        first_intro.set_wrap(true);
        first_intro.set_xalign(0.0);
        first_intro.set_max_width_chars(44);
        first.append(&first_intro);
        let first_label = label("Space name", &["field", "plain", "first-field"]);
        first_label.set_xalign(0.0);
        first.append(&first_label);
        let first_name = gtk::Entry::builder()
            .placeholder_text("Gaming nights")
            .max_length(64)
            .build();
        first_label.set_mnemonic_widget(Some(&first_name));
        first.append(&first_name);
        let first_limit = label(
            "You have reached your space limit.",
            &["muted", "first-limit"],
        );
        first_limit.set_xalign(0.0);
        first.append(&first_limit);
        let first_error = label("", &["error", "first-error"]);
        first_error.set_wrap(true);
        first_error.set_xalign(0.0);
        first.append(&first_error);
        let first_create = gtk::Button::with_label("Create space");
        first_create.add_css_class("primary");
        first_create.add_css_class("first-create");
        first.append(&first_create);
        let first_logout = gtk::Button::with_label("Log out");
        first_logout.add_css_class("flat");
        first_logout.add_css_class("first-logout");
        first_logout.set_halign(gtk::Align::End);
        first.append(&first_logout);
        let first_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&first)
            .build();
        pages.add_named(&first_scroll, Some("first-space"));

        // Chat: rail, sidebar, conversation, then members or a thread.
        let chat = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        // Hosts popovers (`app::popup`): it outlives the rows they point at.
        chat.add_css_class("popover-host");
        let rail: gtk::Box = crate::app::named("GtkBox", "Spaces");
        rail.set_orientation(gtk::Orientation::Vertical);
        rail.set_spacing(8);
        rail.add_css_class("rail");
        chat.append(&rail);

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("sidebar");
        sidebar.set_size_request(220, -1);
        // Expanding children (the account name) must not widen it.
        sidebar.set_hexpand(false);
        // The space's name opens its menu: invite, settings, leave.
        let space_header: gtk::Button = crate::app::named("GtkButton", "Space options");
        space_header.add_css_class("flat");
        space_header.add_css_class("space-header");
        let header_line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let space_name = label("", &["space-name"]);
        space_name.set_ellipsize(pango::EllipsizeMode::End);
        space_name.set_hexpand(true);
        header_line.append(&space_name);
        header_line.append(&images.icon_image(Icon::ChevronDown, "#b9bcbe", 16));
        space_header.set_child(Some(&header_line));
        space_header.set_hexpand(true);
        // Narrow windows: back to the conversation (`app::narrow`).
        let browse_close: gtk::Button = crate::app::named("GtkButton", "Close navigation");
        browse_close.set_child(Some(&images.icon_image(Icon::Close, "#b9bcbe", 18)));
        browse_close.set_tooltip_text(Some("Close navigation"));
        browse_close.add_css_class("flat");
        browse_close.set_valign(gtk::Align::Center);
        browse_close.set_visible(false);
        let space_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        space_row.append(&space_header);
        space_row.append(&browse_close);
        sidebar.append(&space_row);
        let lists = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let channel_header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let channels_toggle: gtk::Button = crate::app::named("GtkButton", "Toggle channels");
        channels_toggle.add_css_class("flat");
        channels_toggle.add_css_class("section-toggle");
        channels_toggle.set_valign(gtk::Align::End);
        let channels_title = label("Channels", &[]);
        let channels_count = label("", &["section-count"]);
        let toggle_line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toggle_line.append(&images.icon_image(Icon::ChevronDown, "#b9bcbe", 12));
        toggle_line.append(&channels_title);
        toggle_line.append(&channels_count);
        channels_toggle.set_child(Some(&toggle_line));
        channels_toggle.set_tooltip_text(Some("Toggle channels"));
        let toggle_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggle_box.add_css_class("section");
        toggle_box.set_hexpand(true);
        toggle_box.append(&channels_toggle);
        channel_header.append(&toggle_box);
        let new_channel: gtk::Button = crate::app::named("GtkButton", "Create channel");
        new_channel.set_child(Some(&images.icon_image(Icon::Plus, "#b9bcbe", 16)));
        new_channel.set_tooltip_text(Some("Create channel"));
        new_channel.add_css_class("flat");
        new_channel.add_css_class("section-action");
        new_channel.set_valign(gtk::Align::End);
        channel_header.append(&new_channel);
        lists.append(&channel_header);
        let channels: gtk::ListBox = crate::app::named("GtkListBox", "Channels");
        channels.add_css_class("channels");
        lists.append(&channels);
        // Pending private channel invitations, with Accept and Decline.
        let channel_invites = gtk::Box::new(gtk::Orientation::Vertical, 6);
        channel_invites.add_css_class("channel-invites");
        lists.append(&channel_invites);
        // As desktop: Direct messages get their own list below a line that
        // resizes the two (`panes`).
        let direct_lists = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let direct_header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let direct_title = label("Direct messages", &["section"]);
        direct_title.set_hexpand(true);
        direct_header.append(&direct_title);
        let new_direct: gtk::Button = crate::app::named("GtkButton", "New direct message");
        new_direct.set_child(Some(&images.icon_image(Icon::Plus, "#b9bcbe", 16)));
        new_direct.set_tooltip_text(Some("New direct message"));
        new_direct.add_css_class("flat");
        new_direct.add_css_class("section-action");
        new_direct.set_valign(gtk::Align::End);
        direct_header.append(&new_direct);
        direct_lists.append(&direct_header);
        // The notifications inbox, with its unread count.
        let inbox = gtk::Button::with_label("Notifications");
        inbox.add_css_class("inbox-button");
        direct_lists.append(&inbox);
        // The Caper team's inbox, for team members only (`app::feedback`).
        let feedback: gtk::ListBox = crate::app::named("GtkListBox", "Feedback");
        feedback.add_css_class("channels");
        feedback.set_visible(false);
        let feedback_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        feedback_line.append(&images.icon_image(Icon::Speech, "#b9bcbe", 16));
        let feedback_name = label("Feedback", &[]);
        feedback_name.set_hexpand(true);
        feedback_line.append(&feedback_name);
        let feedback_waiting = label("", &["count", "mention"]);
        feedback_waiting.set_visible(false);
        feedback_line.append(&feedback_waiting);
        feedback.append(&feedback_line);
        direct_lists.append(&feedback);
        let requests: gtk::ListBox = crate::app::named("GtkListBox", "Message requests");
        requests.add_css_class("channels");
        requests.set_visible(false);
        direct_lists.append(&requests);
        let directs: gtk::ListBox = crate::app::named("GtkListBox", "Direct messages");
        directs.add_css_class("channels");
        direct_lists.append(&directs);
        let scroller = |child: &gtk::Box| {
            gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(child)
                .build()
        };
        let sidebar_lists: gtk::Paned =
            crate::app::named("GtkPaned", "Channel and direct message list height");
        sidebar_lists.set_orientation(gtk::Orientation::Vertical);
        sidebar_lists.add_css_class("caper-split");
        sidebar_lists.set_start_child(Some(&scroller(&lists)));
        sidebar_lists.set_end_child(Some(&scroller(&direct_lists)));
        sidebar_lists.set_shrink_start_child(false);
        sidebar_lists.set_shrink_end_child(false);
        sidebar_lists.set_vexpand(true);
        sidebar.append(&sidebar_lists);
        let dock = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        dock.add_css_class("dock");
        let account_avatar = AvatarView::new(30, true);
        account_avatar.set_presence("online");
        let account_name = label("", &["author"]);
        account_name.set_hexpand(true);
        account_name.set_ellipsize(pango::EllipsizeMode::End);
        // The voice toggles must not widen the sidebar.
        account_name.set_max_width_chars(10);
        let settings: gtk::Button = crate::app::named("GtkButton", "User Settings");
        settings.set_child(Some(&images.icon_image(Icon::Settings, "#b9bcbe", 18)));
        settings.set_tooltip_text(Some("User Settings"));
        settings.add_css_class("flat");
        let mic: gtk::Button = crate::app::named("GtkButton", "Mute");
        mic.set_child(Some(&images.icon_image(Icon::Mic, "#b9bcbe", 18)));
        mic.set_tooltip_text(Some("Mute"));
        mic.add_css_class("flat");
        let deafen: gtk::Button = crate::app::named("GtkButton", "Deafen");
        deafen.set_child(Some(&images.icon_image(Icon::Headphones, "#b9bcbe", 18)));
        deafen.set_tooltip_text(Some("Deafen"));
        deafen.add_css_class("flat");
        let options = |name: &str| {
            let button: gtk::Button = crate::app::named("GtkButton", name);
            let image = images.icon_image(Icon::ChevronDown, "#b9bcbe", 12);
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            button.set_child(Some(&image));
            button.set_tooltip_text(Some(name));
            button.add_css_class("flat");
            button.add_css_class("device-options");
            button
        };
        let input_options = options("Input Options");
        let output_options = options("Output Options");
        dock.append(&account_avatar.root);
        dock.append(&account_name);
        let audio = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        audio.append(&mic);
        audio.append(&input_options);
        audio.append(&deafen);
        audio.append(&output_options);
        dock.append(&audio);
        dock.append(&settings);
        let call_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        call_card.add_css_class("call-card");
        call_card.set_visible(false);
        sidebar.append(&call_card);
        // A known update, above the account bar: opens its review.
        let update_dock = gtk::Button::with_label("Update available");
        update_dock.add_css_class("update-dock");
        update_dock.set_visible(false);
        sidebar.append(&crate::update_shine::shine(&update_dock));
        sidebar.append(&dock);

        let conversation = gtk::Box::new(gtk::Orientation::Vertical, 0);
        conversation.add_css_class("conversation");
        conversation.set_hexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("header");
        // Narrow windows: Browse opens the spaces and channels.
        let browse: gtk::Button = crate::app::named("GtkButton", "Browse");
        browse.set_child(Some(&images.icon_image(Icon::Menu, "#b9bcbe", 18)));
        browse.set_tooltip_text(Some("Browse"));
        browse.add_css_class("flat");
        browse.set_valign(gtk::Align::Center);
        browse.set_visible(false);
        header.append(&browse);
        let channel_avatar = AvatarView::new(28, false);
        channel_avatar.root.set_valign(gtk::Align::Center);
        channel_avatar.root.set_visible(false);
        header.append(&channel_avatar.root);
        let channel_title = label("", &["channel-title"]);
        header.append(&channel_title);
        let status = label("", &["muted", "status"]);
        status.set_hexpand(true);
        header.append(&status);
        // A feedback visit's way back: "Back to feedback" or "Close".
        let visit_back = gtk::Button::with_label("Back to feedback");
        visit_back.set_valign(gtk::Align::Center);
        visit_back.set_visible(false);
        header.append(&visit_back);
        let voice_join = gtk::Button::with_label("Join voice");
        voice_join.add_css_class("primary");
        voice_join.add_css_class("voice-join");
        voice_join.set_valign(gtk::Align::Center);
        voice_join.set_visible(false);
        header.append(&voice_join);
        let pins_button: gtk::Button = crate::app::named("GtkButton", "Pins");
        pins_button.set_child(Some(&images.icon_image(Icon::Pin, "#f3f4f5", 18)));
        pins_button.set_tooltip_text(Some("Pins"));
        pins_button.set_valign(gtk::Align::Center);
        pins_button.add_css_class("flat");
        header.append(&pins_button);
        let members_toggle: gtk::ToggleButton = crate::app::named("GtkToggleButton", "Members");
        members_toggle.set_child(Some(&images.icon_image(Icon::Users, "#f3f4f5", 18)));
        members_toggle.set_tooltip_text(Some("Members"));
        members_toggle.set_active(true);
        members_toggle.set_valign(gtk::Align::Center);
        members_toggle.add_css_class("flat");
        header.append(&members_toggle);
        conversation.append(&header);

        let error = label("", &["error", "banner"]);
        error.set_wrap(true);
        error.set_hexpand(true);
        error.set_xalign(0.0);
        let error_retry = gtk::Button::with_label("Retry");
        error_retry.add_css_class("inline");
        error_retry.set_valign(gtk::Align::Center);
        error_retry.set_visible(false);
        let error_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        error_bar.add_css_class("error-bar");
        error_bar.append(&error);
        error_bar.append(&error_retry);
        error
            .bind_property("visible", &error_bar, "visible")
            .sync_create()
            .build();
        conversation.append(&error_bar);
        let list = List::new("Messages", images, main_rows, 64);
        // As desktop: a short conversation sits by the composer.
        list.view.set_valign(gtk::Align::End);
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
        empty.add_css_class("empty-state");
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_visible(false);
        let empty_icon = images.icon_image(Icon::Hash, "#db6849", 32);
        empty_icon.set_visible(false);
        let empty_title = label("", &["empty-title"]);
        empty_title.set_justify(gtk::Justification::Center);
        empty_title.set_xalign(0.5);
        let empty_note = label("", &["empty-note"]);
        empty_note.set_wrap(true);
        empty_note.set_justify(gtk::Justification::Center);
        empty_note.set_xalign(0.5);
        empty_note.set_max_width_chars(48);
        let empty_action = gtk::Button::with_label("Create channel");
        empty_action.add_css_class("primary");
        empty_action.set_halign(gtk::Align::Center);
        empty_action.set_visible(false);
        empty.append(&empty_icon);
        empty.append(&empty_title);
        empty.append(&empty_note);
        empty.append(&empty_action);
        let list_overlay = gtk::Overlay::new();
        list_overlay.set_child(Some(&list.scroller));
        list_overlay.add_overlay(&empty);
        list_overlay.add_overlay(&composer_lip());
        conversation.append(&list_overlay);
        let newer_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        newer_bar.add_css_class("newer-bar");
        newer_bar.set_halign(gtk::Align::Center);
        newer_bar.set_visible(false);
        let load_newer = gtk::Button::with_label("Load newer messages");
        let latest = gtk::Button::with_label("Back to latest messages");
        newer_bar.append(&load_newer);
        newer_bar.append(&latest);
        conversation.append(&newer_bar);
        let typing = label("", &["typing"]);
        typing.set_ellipsize(pango::EllipsizeMode::End);
        conversation.append(&typing);
        let composer_error = label("", &["error", "composer-error"]);
        composer_error.set_wrap(true);
        conversation.append(&composer_error);
        let compose_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        compose_bar.add_css_class("compose-bar");
        let compose_label = label("", &["muted"]);
        compose_label.set_hexpand(true);
        compose_label.set_ellipsize(pango::EllipsizeMode::End);
        let compose_cancel: gtk::Button = crate::app::named("GtkButton", "Cancel");
        compose_cancel.set_child(Some(&images.icon_image(Icon::Close, "#b9bcbe", 14)));
        compose_cancel.add_css_class("flat");
        let compose_reload = gtk::Button::with_label("Discard draft and load latest");
        compose_reload.add_css_class("flat");
        compose_reload.set_visible(false);
        compose_bar.append(&compose_label);
        compose_bar.append(&compose_reload);
        compose_bar.append(&compose_cancel);
        conversation.append(&compose_bar);
        let request = RequestBar::new();
        conversation.append(&request.waiting);
        let composer = Composer::new(images, false);
        conversation.append(&composer.outer);
        let join_bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        join_bar.add_css_class("join-bar");
        let join_label = label("", &["muted"]);
        join_label.set_hexpand(true);
        join_label.set_wrap(true);
        let join = gtk::Button::with_label("Join channel");
        join.add_css_class("primary");
        join_bar.append(&join_label);
        join_bar.append(&join);
        conversation.append(&join_bar);
        conversation.append(&request.root);
        // Files dragged over the conversation: the shared drop text.
        // The feedback inbox takes the conversation's place (`app::feedback`).
        let feedback_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        feedback_page.add_css_class("conversation");
        feedback_page.set_hexpand(true);
        feedback_page.set_visible(false);
        let stage_body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        stage_body.append(&conversation);
        stage_body.append(&feedback_page);
        let stage = gtk::Overlay::new();
        stage.set_hexpand(true);
        stage.set_child(Some(&stage_body));
        let drop = gtk::Box::new(gtk::Orientation::Vertical, 6);
        drop.add_css_class("drop");
        drop.set_can_target(false);
        drop.set_visible(false);
        let drop_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        drop_card.add_css_class("drop-card");
        drop_card.set_halign(gtk::Align::Center);
        drop_card.set_valign(gtk::Align::Center);
        drop_card.set_vexpand(true);
        drop_card.append(&images.icon_image(Icon::FileUp, "#f3f4f5", 32));
        let drop_title = label("", &["title"]);
        drop_title.set_halign(gtk::Align::Center);
        let drop_detail = label("", &["muted"]);
        drop_detail.set_halign(gtk::Align::Center);
        drop_card.append(&drop_title);
        drop_card.append(&drop_detail);
        drop.append(&drop_card);
        stage.add_overlay(&drop);
        // Narrow windows: the member list slides over the conversation below
        // its header, over a backdrop that closes it (`app::narrow`).
        let members_backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        members_backdrop.add_css_class("drawer-backdrop");
        members_backdrop.set_margin_top(54);
        members_backdrop.set_visible(false);
        stage.add_overlay(&members_backdrop);
        let members_drawer = gtk::Revealer::new();
        members_drawer.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
        members_drawer.set_halign(gtk::Align::End);
        members_drawer.set_margin_top(54);
        members_drawer.add_css_class("members-drawer");
        stage.add_overlay(&members_drawer);

        let members_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        members_panel.add_css_class("members");
        members_panel.set_size_request(220, -1);
        members_panel.set_hexpand(false);
        let members_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        members_header.add_css_class("members-header");
        let members_title = label("Members", &["members-title"]);
        members_header.append(&members_title);
        let members_count = label("", &["muted", "small"]);
        members_count.set_hexpand(true);
        members_count.set_xalign(0.0);
        members_header.append(&members_count);
        let member_search_button: gtk::Button = crate::app::named("GtkButton", "Search members");
        member_search_button.set_child(Some(&images.icon_image(Icon::Search, "#b9bcbe", 16)));
        member_search_button.set_tooltip_text(Some("Search members"));
        member_search_button.add_css_class("flat");
        members_header.append(&member_search_button);
        let members_close: gtk::Button = crate::app::named("GtkButton", "Close member list");
        members_close.set_child(Some(&images.icon_image(Icon::Close, "#b9bcbe", 16)));
        members_close.set_tooltip_text(Some("Close member list"));
        members_close.add_css_class("flat");
        members_header.append(&members_close);
        members_panel.append(&members_header);
        let member_search: gtk::SearchEntry = crate::app::named("GtkSearchEntry", "Search members");
        member_search.set_placeholder_text(Some("Search members"));
        member_search.add_css_class("member-search");
        member_search.set_visible(false);
        members_panel.append(&member_search);
        let members: gtk::ListBox = crate::app::named("GtkListBox", "Members");
        members.set_selection_mode(gtk::SelectionMode::None);
        members.add_css_class("member-list");
        members_panel.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&members)
                .build(),
        );
        let thread_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        thread_panel.add_css_class("thread");
        thread_panel.set_size_request(300, -1);
        thread_panel.set_hexpand(false);
        let thread_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        thread_header.add_css_class("header");
        // As desktop: "Thread", and "in #channel" or "with Name" under it.
        let thread_heading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        thread_heading.set_hexpand(true);
        thread_heading.set_valign(gtk::Align::Center);
        let thread_title = label("Thread", &["thread-title"]);
        thread_title.set_xalign(0.0);
        let thread_place = label("", &["thread-place"]);
        thread_place.set_xalign(0.0);
        thread_place.set_ellipsize(pango::EllipsizeMode::End);
        thread_heading.append(&thread_title);
        thread_heading.append(&thread_place);
        let thread_close: gtk::Button = crate::app::named("GtkButton", "Close thread");
        thread_close.set_child(Some(&images.icon_image(Icon::Close, "#b9bcbe", 18)));
        thread_close.set_valign(gtk::Align::Center);
        thread_close.add_css_class("flat");
        let thread_back = ["Back to channel", "Back to conversation"].map(|name| {
            let button: gtk::Button = crate::app::named("GtkButton", name);
            button.set_child(Some(&images.icon_image(Icon::ArrowLeft, "#b9bcbe", 18)));
            button.set_tooltip_text(Some(name));
            button.set_valign(gtk::Align::Center);
            button.add_css_class("flat");
            button.set_visible(false);
            thread_header.append(&button);
            button
        });
        thread_header.append(&thread_heading);
        thread_header.append(&thread_close);
        thread_panel.append(&thread_header);
        let thread_list = List::new("Thread", images, thread_rows, 0);
        let thread_overlay = gtk::Overlay::new();
        thread_overlay.set_child(Some(&thread_list.scroller));
        thread_overlay.add_overlay(&composer_lip());
        thread_panel.append(&thread_overlay);
        let thread_error = label("", &["error", "composer-error"]);
        thread_error.set_wrap(true);
        thread_panel.append(&thread_error);
        let thread_composer = Composer::new(images, true);
        thread_panel.append(&thread_composer.outer);
        thread_panel.set_visible(false);
        arrive(&thread_panel);
        // As desktop: the thread opens between the conversation and the
        // member list, on a line of its own (`app::panes`).
        let thread_split: gtk::Paned = crate::app::named("GtkPaned", "Thread width");
        thread_split.add_css_class("caper-split");
        thread_split.set_start_child(Some(&stage));
        thread_split.set_end_child(Some(&thread_panel));
        thread_split.set_resize_start_child(true);
        thread_split.set_shrink_start_child(false);
        thread_split.set_resize_end_child(false);
        thread_split.set_shrink_end_child(false);
        // Sidebar | conversation | members, split by draggable lines
        // (`app::panes`). The conversation takes a window's resizing.
        let content_split: gtk::Paned = crate::app::named("GtkPaned", "Member list width");
        content_split.set_start_child(Some(&thread_split));
        content_split.set_end_child(Some(&members_panel));
        content_split.set_resize_start_child(true);
        content_split.set_shrink_start_child(false);
        content_split.set_resize_end_child(false);
        content_split.set_shrink_end_child(false);
        let split: gtk::Paned = crate::app::named("GtkPaned", "Channel sidebar width");
        split.add_css_class("caper-split");
        content_split.add_css_class("caper-split");
        split.set_start_child(Some(&sidebar));
        split.set_end_child(Some(&content_split));
        split.set_resize_start_child(false);
        split.set_shrink_start_child(false);
        split.set_resize_end_child(true);
        split.set_shrink_end_child(false);
        split.set_hexpand(true);
        chat.append(&split);

        // The wide layout must not set the window's minimum width, or a
        // window could never shrink to the narrow one (`crate::narrow`).
        let chat_viewport = gtk::Viewport::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        chat_viewport.set_scroll_to_focus(false);
        chat_viewport.set_child(Some(&chat));
        let chat_width = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&chat_viewport)
            .build();
        pages.add_named(&chat_width, Some("chat"));
        let root = gtk::Overlay::new();
        root.set_child(Some(&pages));
        let zoom_notice = label("", &["zoom-notice"]);
        zoom_notice.set_halign(gtk::Align::End);
        zoom_notice.set_valign(gtk::Align::Start);
        zoom_notice.set_can_target(false);
        zoom_notice.set_visible(false);
        root.add_overlay(&zoom_notice);
        window.set_child(Some(&root));

        Self {
            window,
            zoom_notice,
            pages,
            steps,
            signin_actions: actions,
            email,
            email_button,
            code,
            code_button,
            last_attempt,
            spam_hint,
            resend,
            other_email,
            code_hint,
            signin_status,
            signin_error,
            first_name,
            first_create,
            first_limit,
            first_error,
            first_logout,
            first_inbox,
            rail,
            sidebar,
            browse,
            browse_close,
            members_drawer,
            members_backdrop,
            space_name,
            space_header,
            new_channel,
            channels_toggle,
            channels_count,
            channel_invites,
            channels,
            requests,
            inbox,
            directs,
            account_avatar,
            account_name,
            settings,
            new_direct,
            call_card,
            update_dock,
            mic,
            deafen,
            input_options,
            output_options,
            voice_join,
            visit_back,
            feedback,
            feedback_waiting,
            feedback_page,
            channel_avatar,
            channel_title,
            status,
            pins_button,
            members_toggle,
            members_panel,
            split,
            sidebar_lists,
            content_split,
            members_title,
            members_count,
            member_search_button,
            members_close,
            member_search,
            members,
            list,
            error,
            error_retry,
            empty,
            empty_icon,
            empty_title,
            empty_note,
            empty_action,
            compose_bar,
            compose_label,
            compose_cancel,
            compose_reload,
            composer,
            composer_error,
            typing,
            newer_bar,
            load_newer,
            latest,
            join_bar,
            join_label,
            join,
            request,
            thread_panel,
            thread_split,
            stage,
            thread_place,
            conversation,
            drop,
            drop_title,
            drop_detail,
            thread_close,
            thread_back,
            thread_list,
            thread_composer,
            thread_error,
        }
    }
}

/// What replaces the composer in a DM you can't write to yet: an incoming
/// request ("Name (@user) wants to message you…" with Accept, Decline and
/// Block), someone you blocked ("You blocked @user." with Unblock), or a
/// note over the composer while your own request waits.
pub struct RequestBar {
    pub root: gtk::Box,
    pub text: gtk::Label,
    pub accept: gtk::Button,
    pub decline: gtk::Button,
    pub block: gtk::Button,
    pub unblock: gtk::Button,
    pub error: gtk::Label,
    pub waiting: gtk::Label,
}

impl RequestBar {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("join-bar");
        root.add_css_class("request-bar");
        root.set_visible(false);
        let text = label("", &["muted"]);
        text.set_wrap(true);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let accept = gtk::Button::with_label("Accept");
        accept.add_css_class("primary");
        let decline = gtk::Button::with_label("Decline");
        let block = gtk::Button::with_label("Block");
        let unblock = gtk::Button::with_label("Unblock");
        for button in [&accept, &decline, &block, &unblock] {
            actions.append(button);
        }
        let error = label("", &["error"]);
        error.set_wrap(true);
        error.set_visible(false);
        root.append(&text);
        root.append(&actions);
        root.append(&error);
        let waiting = label("", &["muted", "request-waiting"]);
        waiting.set_wrap(true);
        waiting.set_visible(false);
        Self {
            root,
            text,
            accept,
            decline,
            block,
            unblock,
            error,
            waiting,
        }
    }
}

/// Desktop's sign-in action: eggshell with its label and an arrow, 58px
/// tall, neutral chrome (and a muted arrow) until it can be used. A compact
/// one keeps desktop's 200px minimum.
pub struct LoginAction {
    pub button: gtk::Button,
    label: gtk::Label,
    ready: gtk::Image,
    idle: gtk::Image,
}

impl LoginAction {
    fn new(images: &Images, text: &str, compact: bool) -> Self {
        let button: gtk::Button = crate::app::named("GtkButton", text);
        button.add_css_class("primary");
        button.add_css_class("login-action");
        if compact {
            button.add_css_class("compact");
        }
        button.set_halign(gtk::Align::End);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 32);
        let label = label(text, &[]);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        line.append(&label);
        let ready = images.icon_image(Icon::ArrowRight, "#0c0d0f", 16);
        let idle = images.icon_image(Icon::ArrowRight, "#b9bcbe", 16);
        line.append(&ready);
        line.append(&idle);
        button.set_child(Some(&line));
        let action = Self {
            button,
            label,
            ready,
            idle,
        };
        action.set(text, false);
        action
    }

    pub fn set(&self, text: &str, enabled: bool) {
        self.label.set_label(text);
        self.button.set_sensitive(enabled);
        self.ready.set_visible(enabled);
        self.idle.set_visible(!enabled);
    }
}
