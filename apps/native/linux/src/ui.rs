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

/// A message field: a `GtkTextView` with a placeholder, and a send button
/// drawn with Lucide's `send`.
pub struct Composer {
    pub root: gtk::Box,
    /// The files staged in it (`crate::uploads`).
    pub chips: gtk::Box,
    pub attach: gtk::Button,
    pub view: gtk::TextView,
    pub placeholder: gtk::Label,
    pub send: gtk::Button,
}

impl Composer {
    fn new(images: &Images) -> Self {
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
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.append(&line);
        let attach: gtk::Button = crate::app::named("GtkButton", "Attach files");
        attach.set_child(Some(&images.icon_image(Icon::Plus, "#b9bcbe", 18)));
        attach.set_tooltip_text(Some("Attach files"));
        attach.set_valign(gtk::Align::End);
        attach.add_css_class("flat");
        attach.add_css_class("send");
        line.append(&attach);
        let view: gtk::TextView = crate::app::named("GtkTextView", "Message");
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_accepts_tab(false);
        view.set_hexpand(true);
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
        let send: gtk::Button = crate::app::named("GtkButton", "Send message");
        send.set_child(Some(&images.icon_image(Icon::Send, "#b9bcbe", 18)));
        send.set_tooltip_text(Some("Send"));
        send.set_valign(gtk::Align::End);
        send.add_css_class("flat");
        send.add_css_class("send");
        line.append(&send);
        let empty = placeholder.clone();
        view.buffer()
            .connect_changed(move |buffer| empty.set_visible(buffer.char_count() == 0));
        Self {
            root,
            chips,
            attach,
            view,
            placeholder,
            send,
        }
    }

    pub fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
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
    fn new(name: &str, images: &Rc<Images>, handler: rows::Handler) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let view: gtk::ListView = crate::app::named("GtkListView", name);
        view.set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        view.set_factory(Some(&rows::factory(images.clone(), handler)));
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
    pub pages: gtk::Stack,
    pub steps: gtk::Stack,
    pub email: gtk::Entry,
    pub email_button: gtk::Button,
    pub code: gtk::Entry,
    pub code_button: gtk::Button,
    pub resend: gtk::Button,
    pub other_email: gtk::Button,
    pub code_hint: gtk::Label,
    pub signin_status: gtk::Label,
    pub signin_error: gtk::Label,
    pub rail: gtk::Box,
    pub space_name: gtk::Label,
    pub space_header: gtk::Button,
    pub new_channel: gtk::Button,
    pub channel_invites: gtk::Box,
    pub channels: gtk::ListBox,
    pub directs: gtk::ListBox,
    pub account_avatar: AvatarView,
    pub account_name: gtk::Label,
    pub sign_out: gtk::Button,
    pub settings: gtk::Button,
    pub new_direct: gtk::Button,
    /// The call card above the account bar, and the bar's voice toggles.
    pub call_card: gtk::Box,
    pub mic: gtk::Button,
    pub deafen: gtk::Button,
    /// The header's Join voice, Switch here, Joining… or Leave voice.
    pub voice_join: gtk::Button,
    pub channel_title: gtk::Label,
    pub status: gtk::Label,
    pub pins_button: gtk::Button,
    pub members_toggle: gtk::ToggleButton,
    pub members_panel: gtk::Box,
    pub members_title: gtk::Label,
    pub members_count: gtk::Label,
    pub member_search_button: gtk::Button,
    pub members_close: gtk::Button,
    pub member_search: gtk::SearchEntry,
    pub members: gtk::ListBox,
    pub list: List,
    pub error: gtk::Label,
    pub compose_bar: gtk::Box,
    pub compose_label: gtk::Label,
    pub compose_cancel: gtk::Button,
    pub composer: Composer,
    pub composer_error: gtk::Label,
    pub typing: gtk::Label,
    pub join_bar: gtk::Box,
    pub join_label: gtk::Label,
    pub join: gtk::Button,
    pub thread_panel: gtk::Box,
    pub conversation: gtk::Box,
    pub drop: gtk::Box,
    pub drop_title: gtk::Label,
    pub drop_detail: gtk::Label,
    pub thread_close: gtk::Button,
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

        // Sign-in: email, then the emailed code.
        let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.add_css_class("card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_size_request(400, -1);
        let steps = gtk::Stack::new();
        steps.set_vhomogeneous(false);
        let email_step = gtk::Box::new(gtk::Orientation::Vertical, 12);
        email_step.append(&label("Sign in to Caper", &["title"]));
        let intro = label(
            "Use your email to create an account or log back in.",
            &["muted"],
        );
        intro.set_wrap(true);
        intro.set_max_width_chars(44);
        email_step.append(&intro);
        let email_label = label("Email", &["field"]);
        email_step.append(&email_label);
        let email = gtk::Entry::builder()
            .placeholder_text("you@example.com")
            .input_purpose(gtk::InputPurpose::Email)
            .build();
        // The label names the field for assistive technologies.
        email_label.set_mnemonic_widget(Some(&email));
        email_step.append(&email);
        let email_button = gtk::Button::with_label("Continue");
        email_button.add_css_class("primary");
        email_step.append(&email_button);
        steps.add_named(&email_step, Some("email"));
        let code_step = gtk::Box::new(gtk::Orientation::Vertical, 12);
        code_step.append(&label("Check your email", &["title"]));
        let code_hint = label("", &["muted"]);
        code_hint.set_wrap(true);
        // Wrapped labels ask for their whole text's width; keep the card's.
        code_hint.set_max_width_chars(44);
        code_step.append(&code_hint);
        let code_label = label("Sign-in code", &["field"]);
        code_step.append(&code_label);
        let code = gtk::Entry::builder()
            .placeholder_text("e.g. ABC234")
            .build();
        code_label.set_mnemonic_widget(Some(&code));
        code_step.append(&code);
        let code_button = gtk::Button::with_label("Continue");
        code_button.add_css_class("primary");
        code_step.append(&code_button);
        let links = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        links.set_homogeneous(true);
        let resend = gtk::Button::with_label("Resend code");
        resend.add_css_class("flat");
        let other_email = gtk::Button::with_label("Use a different email");
        other_email.add_css_class("flat");
        links.append(&resend);
        links.append(&other_email);
        code_step.append(&links);
        steps.add_named(&code_step, Some("code"));
        card.append(&steps);
        let signin_status = label("", &["muted"]);
        signin_status.set_wrap(true);
        card.append(&signin_status);
        let signin_error = label("", &["error"]);
        signin_error.set_wrap(true);
        card.append(&signin_error);
        let signin = gtk::Box::new(gtk::Orientation::Vertical, 0);
        signin.add_css_class("signin");
        signin.append(&card);
        card.set_vexpand(true);
        pages.add_named(&signin, Some("signin"));

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
        sidebar.set_size_request(260, -1);
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
        sidebar.append(&space_header);
        let lists = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let channel_header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let channels_title = label("Channels", &["section"]);
        channels_title.set_hexpand(true);
        channel_header.append(&channels_title);
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
        lists.append(&direct_header);
        let directs: gtk::ListBox = crate::app::named("GtkListBox", "Direct messages");
        directs.add_css_class("channels");
        lists.append(&directs);
        sidebar.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&lists)
                .build(),
        );
        let dock = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        dock.add_css_class("dock");
        let account_avatar = AvatarView::new(32, true);
        account_avatar.set_presence("online");
        let account_name = label("", &["author"]);
        account_name.set_hexpand(true);
        account_name.set_ellipsize(pango::EllipsizeMode::End);
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
        let sign_out: gtk::Button = crate::app::named("GtkButton", "Sign out");
        sign_out.set_child(Some(&images.icon_image(Icon::LogOut, "#b9bcbe", 18)));
        sign_out.set_tooltip_text(Some("Sign out"));
        sign_out.add_css_class("flat");
        dock.append(&account_avatar.root);
        dock.append(&account_name);
        dock.append(&mic);
        dock.append(&deafen);
        dock.append(&settings);
        dock.append(&sign_out);
        let call_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        call_card.add_css_class("call-card");
        call_card.set_visible(false);
        sidebar.append(&call_card);
        sidebar.append(&dock);
        chat.append(&sidebar);

        let conversation = gtk::Box::new(gtk::Orientation::Vertical, 0);
        conversation.add_css_class("conversation");
        conversation.set_hexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("header");
        let channel_title = label("", &["channel-title"]);
        header.append(&channel_title);
        let status = label("", &["muted", "status"]);
        status.set_hexpand(true);
        header.append(&status);
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
        conversation.append(&error);
        let list = List::new("Messages", images, main_rows);
        conversation.append(&list.scroller);
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
        compose_bar.append(&compose_label);
        compose_bar.append(&compose_cancel);
        conversation.append(&compose_bar);
        let composer = Composer::new(images);
        conversation.append(&composer.root);
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
        // Files dragged over the conversation: the shared drop text.
        let stage = gtk::Overlay::new();
        stage.set_hexpand(true);
        stage.set_child(Some(&conversation));
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
        chat.append(&stage);

        let members_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        members_panel.add_css_class("members");
        members_panel.set_size_request(240, -1);
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
        let member_search = gtk::SearchEntry::new();
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
        chat.append(&members_panel);

        let thread_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        thread_panel.add_css_class("thread");
        thread_panel.set_size_request(420, -1);
        thread_panel.set_hexpand(false);
        let thread_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        thread_header.add_css_class("header");
        let thread_title = label("Thread", &["channel-title"]);
        thread_title.set_hexpand(true);
        let thread_close: gtk::Button = crate::app::named("GtkButton", "Close thread");
        thread_close.set_child(Some(&images.icon_image(Icon::Close, "#b9bcbe", 18)));
        thread_close.set_valign(gtk::Align::Center);
        thread_close.add_css_class("flat");
        thread_header.append(&thread_title);
        thread_header.append(&thread_close);
        thread_panel.append(&thread_header);
        let thread_list = List::new("Thread", images, thread_rows);
        thread_panel.append(&thread_list.scroller);
        let thread_error = label("", &["error", "composer-error"]);
        thread_error.set_wrap(true);
        thread_panel.append(&thread_error);
        let thread_composer = Composer::new(images);
        thread_panel.append(&thread_composer.root);
        thread_panel.set_visible(false);
        chat.append(&thread_panel);
        pages.add_named(&chat, Some("chat"));
        window.set_child(Some(&pages));

        Self {
            window,
            pages,
            steps,
            email,
            email_button,
            code,
            code_button,
            resend,
            other_email,
            code_hint,
            signin_status,
            signin_error,
            rail,
            space_name,
            space_header,
            new_channel,
            channel_invites,
            channels,
            directs,
            account_avatar,
            account_name,
            sign_out,
            settings,
            new_direct,
            call_card,
            mic,
            deafen,
            voice_join,
            channel_title,
            status,
            pins_button,
            members_toggle,
            members_panel,
            members_title,
            members_count,
            member_search_button,
            members_close,
            member_search,
            members,
            list,
            error,
            compose_bar,
            compose_label,
            compose_cancel,
            composer,
            composer_error,
            typing,
            join_bar,
            join_label,
            join,
            thread_panel,
            conversation,
            drop,
            drop_title,
            drop_detail,
            thread_close,
            thread_list,
            thread_composer,
            thread_error,
        }
    }
}
