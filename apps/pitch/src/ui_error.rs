use eframe::egui::Ui;

pub trait UiError {
    fn ui_error(&self, ui: &mut Ui);
}
