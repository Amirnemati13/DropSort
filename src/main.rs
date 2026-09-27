#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dropsort::{self, MovedFile, PlannedMove};
use eframe::egui::{self, Color32, RichText};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

const ACCENT: Color32 = Color32::from_rgb(116, 224, 190);
enum JobResult {
    Sort(dropsort::BatchResult),
    Undo(Vec<MovedFile>, Vec<String>),
}

struct DropSort {
    files: Vec<PathBuf>,
    root: Option<PathBuf>,
    preview: Vec<PlannedMove>,
    history: Vec<MovedFile>,
    messages: Vec<String>,
    status: String,
    job: Option<Receiver<JobResult>>,
    confirm: bool,
}

impl DropSort {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(12.0, 12.0);
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
        style.visuals.panel_fill = Color32::from_rgb(17, 23, 31);
        style.visuals.window_fill = Color32::from_rgb(25, 33, 44);
        style.visuals.selection.bg_fill = Color32::from_rgb(38, 100, 85);
        cc.egui_ctx.set_style(style);
        let files = std::env::args_os().skip(1).map(PathBuf::from).collect();
        Self {
            files,
            root: None,
            preview: Vec::new(),
            history: Vec::new(),
            messages: Vec::new(),
            status: "Ready when you are".into(),
            job: None,
            confirm: false,
        }
    }

    fn refresh(&mut self) {
        self.preview.clear();
        self.messages.clear();
        if let Some(root) = &self.root {
            match dropsort::plan(&self.files, root) {
                Ok(p) => {
                    self.preview = p;
                    self.status = format!("{} files ready to organize", self.preview.len());
                }
                Err(e) => {
                    self.messages.push(e.to_string());
                    self.status = "Check your selection".into();
                }
            }
        }
    }

    fn add(&mut self, files: Vec<PathBuf>) {
        for f in files {
            if !self.files.contains(&f) {
                self.files.push(f);
            }
        }
        self.refresh();
    }

    fn start_sort(&mut self, ctx: &egui::Context) {
        let items = self.preview.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.job = Some(rx);
        self.status = "Organizing your files...".into();
        thread::spawn(move || {
            let _ = tx.send(JobResult::Sort(dropsort::execute(&items)));
            ctx.request_repaint();
        });
    }

    fn start_undo(&mut self, ctx: &egui::Context) {
        let mut history = std::mem::take(&mut self.history);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.job = Some(rx);
        self.status = "Restoring original locations...".into();
        thread::spawn(move || {
            let errors = dropsort::undo(&mut history);
            let _ = tx.send(JobResult::Undo(history, errors));
            ctx.request_repaint();
        });
    }
}

fn display_path(p: &std::path::Path) -> String {
    p.display()
        .to_string()
        .trim_start_matches(r"\\?\")
        .to_owned()
}

impl eframe::App for DropSort {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.job = None;
                    match result {
                        JobResult::Sort(result) => {
                            let n = result.moved.len();
                            self.files.retain(|f| f.exists());
                            self.refresh();
                            if n > 0 {
                                self.history = result.moved;
                            }
                            self.messages = result.errors;
                            self.status = format!(
                                "{n} files organized. {}",
                                if self.messages.is_empty() {
                                    "Your space, a little calmer."
                                } else {
                                    "Stopped on an error; successful moves can be undone."
                                }
                            );
                        }
                        JobResult::Undo(history, errors) => {
                            self.history = history;
                            self.refresh();
                            self.messages = errors;
                            self.status = if self.history.is_empty() {
                                "Files restored to their original locations.".into()
                            } else {
                                "Some files could not be restored. Resolve the issue and retry Undo.".into()
                            };
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    self.messages.push(
                        "The worker stopped unexpectedly. Inspect your files before continuing."
                            .into(),
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let busy = self.job.is_some();
        if busy && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if !busy && !self.confirm {
            let dropped = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .filter_map(|f| f.path.clone())
                    .collect::<Vec<_>>()
            });
            if !dropped.is_empty() {
                self.add(dropped);
            }
        }

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                } else {
                    ui.colored_label(ACCENT, "●");
                }
                ui.label(&self.status);
            });
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().show(ctx,|ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("DropSort").size(34.0).strong().color(ACCENT));
                ui.label(RichText::new("A place for every file.").size(16.0).color(Color32::GRAY));
            });
            ui.add_space(12.0);
            ui.add_enabled_ui(!busy && !self.confirm,|ui| {
                egui::Frame::group(ui.style()).inner_margin(20.0).show(ui,|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| { ui.label(RichText::new("01  /  Add your files").size(20.0).strong()); ui.label("Drop files anywhere in this window, or browse to select them."); });
                        if ui.button("+  Choose files").clicked() { if let Some(files)=rfd::FileDialog::new().set_title("Choose files to organize").pick_files() { self.add(files); } }
                    });
                    if !self.files.is_empty() {
                        ui.horizontal(|ui| { ui.label(format!("{} selected",self.files.len())); if ui.small_button("Clear selection").clicked() { self.files.clear(); self.refresh(); } });
                        egui::ScrollArea::horizontal().max_height(44.0).show(ui,|ui| { ui.horizontal(|ui| { let mut remove=None; for (i,f) in self.files.iter().enumerate() { if ui.small_button(format!("{}  ×",f.file_name().unwrap_or_default().to_string_lossy())).on_hover_text(display_path(f)).clicked() { remove=Some(i); } } if let Some(i)=remove { self.files.remove(i); self.refresh(); } }); });
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("02  /  Choose a home").size(20.0).strong());
                    if ui.button("Choose destination").clicked() { if let Some(root)=rfd::FileDialog::new().set_title("Choose the parent folder for your categories").pick_folder() { self.root=Some(root); self.refresh(); } }
                });
                ui.label(self.root.as_ref().map(|p| display_path(p)).unwrap_or_else(|| "Select a folder. Category folders will be created inside it.".into()));
                ui.horizontal_wrapped(|ui| { for c in dropsort::CATEGORIES { ui.colored_label(ACCENT,c); } });
                ui.separator();
                ui.horizontal(|ui| { ui.label(RichText::new("03  /  Review & organize").size(20.0).strong()); if ui.small_button("Refresh preview").clicked() { self.refresh(); } });
                ui.label("Existing files stay safe. Duplicate names receive a number.");
                egui::ScrollArea::vertical().id_salt("preview").max_height((ui.available_height()-160.0).max(90.0)).show(ui,|ui| {
                    if self.preview.is_empty() { ui.add_space(20.0); ui.label(RichText::new(if self.files.is_empty() { "Your next clean-up starts with a single drop." } else if self.root.is_none() { "Choose a destination to see the preview." } else { "No moves to preview. Files may already be sorted; check any errors below." }).color(Color32::GRAY)); ui.add_space(20.0); }
                    for item in &self.preview {
                        egui::Frame::group(ui.style()).show(ui,|ui| { ui.set_width(ui.available_width()); ui.horizontal(|ui| { ui.strong(item.source.file_name().unwrap_or_default().to_string_lossy()); ui.colored_label(ACCENT,item.category); ui.label(format!("{:.1} KB",item.bytes as f64/1024.0)); }); ui.small(format!("From  {}",display_path(&item.source))); ui.label(format!("To       {}",display_path(&item.destination))); });
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.preview.is_empty() && self.messages.is_empty(),egui::Button::new(RichText::new(format!("Organize {} files",self.preview.len())).color(Color32::from_rgb(17,23,31)).strong()).fill(ACCENT)).clicked() { self.confirm=true; }
                    if ui.add_enabled(!self.history.is_empty(),egui::Button::new("Undo last operation")).clicked() { self.start_undo(ctx); }
                });
                ui.small("Undo is available in this session, until the next successful sort. Keep DropSort open to undo.");
            });
            if !self.messages.is_empty() {
                egui::ScrollArea::vertical().id_salt("errors").max_height(85.0).show(ui,|ui| { for msg in &self.messages { ui.colored_label(Color32::from_rgb(255,181,134),msg); } });
            }
        });

        if self.confirm {
            egui::Window::new("Ready to organize?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Move {} files to the destinations shown in the preview?",
                        self.preview.len()
                    ));
                    if !self.history.is_empty() {
                        ui.label(
                            "This replaces the previous Undo history after a successful move.",
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Move files").clicked() {
                            self.confirm = false;
                            self.start_sort(ctx);
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm = false;
                        }
                    });
                });
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("drop"),
            ));
            let rect = ctx.screen_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(220));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to add files",
                egui::FontId::proportional(32.0),
                ACCENT,
            );
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 760.0])
            .with_min_inner_size([800.0, 680.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "DropSort",
        options,
        Box::new(|cc| Ok(Box::new(DropSort::new(cc)))),
    )
}
