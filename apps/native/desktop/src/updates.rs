//! Self-updates (`caper_client::updates`), redrawn through egui.

pub use caper_client::updates::*;
use eframe::egui;

/// Starts background checks when this is a release build with an updater.
pub fn start(context: &egui::Context, enabled: bool) -> Updates {
    let context = context.clone();
    Updates::start(move || context.request_repaint(), enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestPass as _;

    fn release(version: &str) -> Available {
        Available {
            version: version.into(),
            notes: "Release notes".into(),
            changelog: Vec::new(),
            history_complete: false,
            highlights: None,
            released: None,
            revision: None,
            sequence: None,
            last_error: None,
            can_apply: true,
        }
    }

    #[test]
    fn settings_check_reports_progress_success_and_retryable_failure_in_place() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = crate::CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.updates = Updates::preview(release("0.1.98"));
        app.updates.dismiss();
        let receiver = app.updates.watch_checks();
        let frame = |app: &mut crate::CaperApp, events| {
            context.test_pass(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| {
                    context.root_ui(|root| {
                        egui::CentralPanel::default().show(root, |ui| app.settings_dialog(ui))
                    });
                },
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == "Check for updates"
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("manual check stays available in Settings");
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        receiver
            .try_recv()
            .expect("the button wakes the update worker");
        let checking = frame(&mut app, vec![]);
        let button = checking
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Check for updates"))
            .unwrap();
        assert!(button.1.is_disabled(), "a second check must not overlap");
        let checking_bounds = button.1.bounds().unwrap();
        for (result, expected) in [
            (Ok(None), "No newer published release is available."),
            (
                Err("Offline".into()),
                "Could not check for updates: Offline",
            ),
        ] {
            app.updates.finish_check(result);
            frame(&mut app, vec![]);
            let output = frame(&mut app, vec![]);
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Text(text) if text.galley.job.text == expected
                )),
                "missing check feedback: {expected}"
            );
            let button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Check for updates"))
                .unwrap();
            assert!(
                !button.1.is_disabled(),
                "success and failure allow another check"
            );
            let bounds = button.1.bounds().unwrap();
            assert_eq!(
                bounds, checking_bounds,
                "a fast check must not move the action"
            );
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            receiver.try_recv().expect("retry wakes the update worker");
            assert!(app.dialog.is_none());
        }
        app.updates.finish_check(Ok(Some(release("0.1.99"))));
        frame(&mut app, vec![]);
        assert_eq!(app.updates.available().unwrap().version, "0.1.99");
        assert!(app.dialog.is_none());
    }
}
