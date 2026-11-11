//! `reportcraft`: the desktop designer.
//!
//! `reportcraft [REPORT.rcr.json] [--data FILE.csv] [--sample NAME] [--screenshot FILE.ppm]`:
//! with no argument it opens the built-in sample, so there is something to look at straight away.
//! `--screenshot` draws a few frames, saves the window as a PPM picture and quits (for checks
//! and documentation). `--control PORT` lets scripts and agents drive the window (see
//! `control.rs` and `reportcraft-cli send`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control;

use std::path::{Path, PathBuf};

use reportcraft_engine::Session;
use reportcraft_ui_egui::{Designer, Dialogs};

/// The system's file dialogs.
struct SystemDialogs;

impl Dialogs for SystemDialogs {
    fn open(&self, title: &str, extensions: &[&str]) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).add_filter(extensions.join(", "), extensions).pick_file()
    }

    fn save(&self, title: &str, extension: &str, name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).add_filter(extension, &[extension]).set_file_name(name).save_file()
    }
}

struct App {
    designer: Designer,
    screenshot: Option<Screenshot>,
    /// Requests from the control channel, when it is on.
    control: Option<std::sync::mpsc::Receiver<control::Request>>,
    /// The token file to remove on exit.
    token_file: Option<PathBuf>,
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(f) = &self.token_file {
            let _ = std::fs::remove_file(f);
        }
    }
}

impl App {
    /// Answers the control channel's queued requests, between two frames.
    fn answer_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.control else { return };
        let requests: Vec<control::Request> = rx.try_iter().collect();
        for r in requests {
            match r.method.as_str() {
                "app.quit" => {
                    let _ = r.reply.send(Ok(serde_json::json!({"quitting": true})));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                "ui.screenshot" => match r.params.get("path").and_then(serde_json::Value::as_str) {
                    Some(path) => {
                        let (path, reply, repaint) = (PathBuf::from(path), r.reply, ctx.clone());
                        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                            let answer = write_ppm(&path, &image)
                                .map(|()| serde_json::json!({"path": path.display().to_string(), "width": image.size[0], "height": image.size[1]}))
                                .map_err(|e| format!("{}: {e}", path.display()));
                            let _ = reply.send(answer);
                            repaint.request_repaint();
                        })));
                    }
                    None => {
                        let _ = r.reply.send(Err("ui.screenshot needs `path` (a .ppm file)".into()));
                    }
                },
                method => {
                    let _ = r.reply.send(self.designer.control(method, &r.params));
                }
            }
        }
        ctx.request_repaint();
    }
}

/// A screenshot to take once the window has settled, then quit.
struct Screenshot {
    path: PathBuf,
    frames: u32,
    requested: bool,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Frames drawn before the screenshot, so fonts and layout have settled.
const SETTLE_FRAMES: u32 = 10;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.answer_control(&ui.ctx().clone());
        self.designer.ui(ui);
        let Some(shot) = self.screenshot.as_mut() else { return };
        let ctx = ui.ctx().clone();
        if shot.done.load(std::sync::atomic::Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        shot.frames += 1;
        if shot.frames >= SETTLE_FRAMES && !shot.requested {
            shot.requested = true;
            let (path, done, repaint) = (shot.path.clone(), shot.done.clone(), ctx.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                if let Err(e) = write_ppm(&path, &image) {
                    eprintln!("{}: {e}", path.display());
                }
                done.store(true, std::sync::atomic::Ordering::SeqCst);
                repaint.request_repaint();
            })));
        }
        ctx.request_repaint();
    }
}

/// The picture as a binary PPM (`P6`): no image library needed.
fn write_ppm(path: &Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in &image.pixels {
        out.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    std::fs::write(path, out)
}

/// What the command line asks to open.
#[derive(Debug, Default, PartialEq)]
struct Args {
    report: Option<PathBuf>,
    data: Option<PathBuf>,
    sample: Option<String>,
    screenshot: Option<PathBuf>,
    control: Option<u16>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data" => out.data = Some(PathBuf::from(it.next().ok_or("`--data` needs a CSV file")?)),
            "--sample" => out.sample = Some(it.next().ok_or("`--sample` needs a name")?.clone()),
            "--screenshot" => out.screenshot = Some(PathBuf::from(it.next().ok_or("`--screenshot` needs a file")?)),
            "--control" => {
                let port = it.next().ok_or("`--control` needs a port")?;
                out.control = Some(port.parse().map_err(|_| format!("`{port}` is not a port number"))?);
            }
            "--help" | "-h" => {
                return Err("reportcraft [REPORT.rcr.json] [--data FILE.csv] [--sample NAME] [--screenshot FILE.ppm] [--control PORT]".into());
            }
            path if !path.starts_with("--") => out.report = Some(PathBuf::from(path)),
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    Ok(out)
}

/// The designer with what the command line asked for (the sample when it asked for nothing).
fn designer(args: &Args) -> Designer {
    let mut d = Designer::new(Session::default(), Box::new(SystemDialogs));
    match (&args.sample, &args.report) {
        (Some(name), _) => d.open_sample(name),
        (None, None) => d.open_sample("rentals"),
        (None, Some(_)) => {}
    }
    if let Some(report) = &args.report {
        d.open_report(Path::new(report));
    }
    if let Some(csv) = &args.data {
        d.load_csv(csv);
    }
    d
}

fn main() -> eframe::Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    let viewport = egui::ViewportBuilder::default().with_title("ReportCraft").with_inner_size([1400.0, 900.0]).with_app_id("reportcraft");
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let designer = designer(&args);
    let screenshot = args.screenshot.clone().map(|path| Screenshot { path, frames: 0, requested: false, done: Default::default() });
    let control_port = args.control;
    eframe::run_native(
        "ReportCraft",
        options,
        Box::new(move |cc| {
            let mut app = App { designer, screenshot, control: None, token_file: None };
            if let Some(port) = control_port {
                let token = reportcraft_engine::control::new_token();
                let ctx = cc.egui_ctx.clone();
                match control::start(port, token.clone(), move || ctx.request_repaint()) {
                    Ok((port, rx)) => match reportcraft_engine::control::write_token(port, &token) {
                        Ok(file) => {
                            eprintln!("control channel on 127.0.0.1:{port} (token in {})", file.display());
                            app.control = Some(rx);
                            app.token_file = Some(file);
                        }
                        Err(e) => eprintln!("control channel: can't write its token: {e}"),
                    },
                    Err(e) => eprintln!("control channel: can't listen on port {port}: {e}"),
                }
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_name_a_report_data_or_a_sample() {
        let a = |v: &[&str]| parse(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(a(&[]), Ok(Args::default()));
        assert_eq!(
            a(&["r.rcr.json", "--data", "d.csv"]),
            Ok(Args { report: Some("r.rcr.json".into()), data: Some("d.csv".into()), sample: None, screenshot: None, control: None })
        );
        assert_eq!(a(&["--sample", "rentals"]).map(|x| x.sample), Ok(Some("rentals".into())));
        assert_eq!(a(&["--control", "7979"]).map(|x| x.control), Ok(Some(7979)));
        assert!(a(&["--control", "port"]).is_err());
        assert!(a(&["--data"]).is_err());
        assert!(a(&["--nope"]).is_err());
    }
}
