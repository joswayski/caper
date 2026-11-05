//! Keep label selection intact while a context menu uses the pointer.
//! egui treats every pointer button as a selection gesture. Its label
//! selection is opaque, so preserve it and remember the text egui reports
//! when the selection settles.
use eframe::egui::{self, text_selection::LabelSelectionState};

#[derive(Default)]
pub struct MessageSelection {
    saved: Option<LabelSelectionState>,
    /// The selected text, as of its last `TextSelectionSettled` report.
    settled: String,
    /// The selection when the context menu opened.
    pub text: String,
}

impl egui::Plugin for MessageSelection {
    fn debug_name(&self) -> &'static str {
        "MessageSelection"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let secondary = ctx.input(|i| i.pointer.secondary_down() || i.pointer.secondary_clicked());
        let protect = secondary || egui::Popup::is_any_open(ctx);
        self.saved = protect.then(|| ctx.plugin::<LabelSelectionState>().lock().clone());
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary)) {
            self.text.clone_from(&self.settled);
        } else if !protect {
            self.text.clear();
        }
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::unique("protect-message-selection"), protect)
        });
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
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
        ctx.output(|output| {
            for command in &output.commands {
                if let egui::OutputCommand::TextSelectionSettled(text) = command {
                    self.settled.clone_from(text);
                }
            }
        });
        if !ctx.plugin::<LabelSelectionState>().lock().has_selection() {
            self.settled.clear();
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
        data.get_temp::<bool>(egui::Id::unique("protect-message-selection"))
            .unwrap_or(false)
    });
    let pointer = protect.then(|| ui.input_mut(|i| std::mem::take(&mut i.pointer)));
    LabelSelectionState::label_text_selection(
        ui,
        response,
        position,
        galley,
        ui.visuals().text_color(),
        egui::Stroke::NONE,
    );
    if let Some(pointer) = pointer {
        ui.input_mut(|i| i.pointer = pointer);
    }
}
