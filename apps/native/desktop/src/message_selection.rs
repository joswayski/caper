//! Keep label selection intact while a context menu uses the pointer.
//! egui 0.33 treats every pointer button as a selection gesture. Its label
//! selection is opaque, so preserve it and let egui extract the selected text.
use eframe::egui::{self, text_selection::LabelSelectionState};

#[derive(Default)]
pub struct MessageSelection {
    saved: Option<LabelSelectionState>,
    capture: bool,
    pub text: String,
}

impl egui::Plugin for MessageSelection {
    fn debug_name(&self) -> &'static str {
        "MessageSelection"
    }

    fn on_begin_pass(&mut self, ctx: &egui::Context) {
        let secondary = ctx.input(|i| i.pointer.secondary_down() || i.pointer.secondary_clicked());
        let protect = secondary || egui::Popup::is_any_open(ctx);
        self.saved = protect.then(|| ctx.plugin::<LabelSelectionState>().lock().clone());
        self.capture = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
        if self.capture || !protect {
            self.text.clear();
        }
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("protect-message-selection"), protect));
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("capture-message-selection"), self.capture)
        });
    }

    fn on_end_pass(&mut self, ctx: &egui::Context) {
        // Registered after egui's selection plugin: its end-pass cleanup must
        // not discard selection when the pointer presses a menu item.
        if let Some(saved) = self.saved.take() {
            *ctx.plugin::<LabelSelectionState>().lock() = saved;
        } else if ctx.input(|i| {
            i.pointer.primary_clicked()
                && !i
                    .pointer
                    .button_double_clicked(egui::PointerButton::Primary)
                && !i
                    .pointer
                    .button_triple_clicked(egui::PointerButton::Primary)
                && !i.modifiers.shift
        }) {
            // A plain click leaves egui an empty cursor range, whose Copy
            // fallback is the entire label. That is not selected text.
            ctx.plugin::<LabelSelectionState>().lock().clear_selection();
        }
        if self.capture {
            ctx.output_mut(|output| {
                output.commands.retain(|command| {
                    if let egui::OutputCommand::CopyText(text) = command {
                        self.text.clone_from(text);
                        false
                    } else {
                        true
                    }
                });
            });
        }
    }
}

pub fn paint(
    ui: &egui::Ui,
    response: &egui::Response,
    position: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
) {
    let protect = ui.data(|data| {
        data.get_temp::<bool>(egui::Id::new("protect-message-selection"))
            .unwrap_or(false)
    });
    let pointer = protect.then(|| ui.input_mut(|i| std::mem::take(&mut i.pointer)));
    // Scope the synthetic copy to message labels, never the composer or other
    // editable fields. It is captured at end-pass, not sent to the clipboard.
    let capture = ui.data(|data| {
        data.get_temp::<bool>(egui::Id::new("capture-message-selection"))
            .unwrap_or(false)
    });
    if capture {
        ui.input_mut(|i| i.events.push(egui::Event::Copy));
    }
    LabelSelectionState::label_text_selection(
        ui,
        response,
        position,
        galley,
        ui.visuals().text_color(),
        egui::Stroke::NONE,
    );
    if capture {
        ui.input_mut(|i| {
            i.events.pop();
        });
    }
    if let Some(pointer) = pointer {
        ui.input_mut(|i| i.pointer = pointer);
    }
}
