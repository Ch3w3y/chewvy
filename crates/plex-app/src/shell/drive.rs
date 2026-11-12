//! Test driver for visual/functional audits: only active when
//! `PLEX_APP_DRIVE=<dir>` is set. Each frame it takes `<dir>/cmd` (one command
//! per line), injects the input into egui, and writes screenshots as PPM.
//!
//! Commands (coordinates are physical window pixels, as in a screenshot):
//! `click X Y`, `move X Y`, `scroll DX DY`, `key NAME` (egui key name, e.g.
//! `Enter`, `Escape`, `ArrowDown`, optional `ctrl+`/`shift+`/`alt+` prefix),
//! `text STRING`, `wait N` (frames), `shot NAME` (writes `<dir>/NAME.ppm`).

#![allow(unsafe_code)]

use std::collections::VecDeque;
use std::path::PathBuf;

/// Environment variable that enables the driver.
pub const DRIVE_ENV: &str = "PLEX_APP_DRIVE";

/// Pending audit commands for one window.
pub struct Driver {
    dir: PathBuf,
    queue: VecDeque<String>,
    wait: u32,
    shot: Option<String>,
}

impl Driver {
    /// The driver when `PLEX_APP_DRIVE` names a directory.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os(DRIVE_ENV)?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            dir,
            queue: VecDeque::new(),
            wait: 0,
            shot: None,
        })
    }

    fn take_commands(&mut self) {
        let cmd = self.dir.join("cmd");
        let taken = self.dir.join("cmd.taken");
        if std::fs::rename(&cmd, &taken).is_ok() {
            if let Ok(text) = std::fs::read_to_string(&taken) {
                self.queue.extend(
                    text.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned),
                );
            }
        }
    }

    /// Inject at most one command's input before the egui frame runs.
    pub fn before_frame(&mut self, input: &mut egui::RawInput, pixels_per_point: f32) {
        self.take_commands();
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        let Some(line) = self.queue.pop_front() else {
            return;
        };
        let mut parts = line.splitn(2, ' ');
        let verb = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default();
        let nums: Vec<f32> = rest
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .collect();
        let ppp = pixels_per_point.max(0.1);
        let pos = |i: usize| {
            egui::pos2(
                nums.get(i).copied().unwrap_or(0.0) / ppp,
                nums.get(i + 1).copied().unwrap_or(0.0) / ppp,
            )
        };
        let events = &mut input.events;
        match verb {
            "move" => events.push(egui::Event::PointerMoved(pos(0))),
            "click" => {
                let p = pos(0);
                events.push(egui::Event::PointerMoved(p));
                for pressed in [true, false] {
                    events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                self.wait = 2;
            }
            "scroll" => events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(
                    nums.first().copied().unwrap_or(0.0),
                    nums.get(1).copied().unwrap_or(0.0),
                ),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }),
            "key" => {
                let mut modifiers = egui::Modifiers::NONE;
                let mut name = rest.trim();
                while let Some((prefix, tail)) = name.split_once('+') {
                    match prefix {
                        "ctrl" => modifiers.ctrl = true,
                        "shift" => modifiers.shift = true,
                        "alt" => modifiers.alt = true,
                        _ => break,
                    }
                    name = tail;
                }
                if modifiers.ctrl {
                    modifiers.command = true;
                }
                if let Some(key) = egui::Key::from_name(name) {
                    for pressed in [true, false] {
                        events.push(egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed,
                            repeat: false,
                            modifiers,
                        });
                    }
                }
                self.wait = 2;
            }
            "text" => events.push(egui::Event::Text(rest.to_owned())),
            "wait" => self.wait = rest.trim().parse().unwrap_or(10),
            "shot" => self.shot = Some(rest.trim().to_owned()),
            _ => log::warn!("drive: unknown command {verb:?}"),
        }
    }

    /// After painting and before the swap: write a requested screenshot.
    pub fn after_paint(&mut self, gl: &glow::Context, size: [u32; 2]) {
        let Some(name) = self.shot.take() else {
            return;
        };
        let [w, h] = size;
        let (Ok(wi), Ok(hi)) = (i32::try_from(w), i32::try_from(h)) else {
            return;
        };
        let row = (w as usize) * 4;
        let mut rgba = vec![0_u8; row * h as usize];
        // SAFETY: the buffer holds exactly w*h RGBA pixels and the GL context
        // is current on this thread during the frame.
        unsafe {
            use glow::HasContext as _;
            gl.read_pixels(
                0,
                0,
                wi,
                hi,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut rgba)),
            );
        }
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for line in rgba.chunks(row).rev() {
            for px in line.as_chunks::<4>().0 {
                ppm.extend_from_slice(&[px[0], px[1], px[2]]);
            }
        }
        let tmp = self.dir.join(format!("{name}.ppm.part"));
        if std::fs::write(&tmp, ppm).is_ok() {
            let _ = std::fs::rename(&tmp, self.dir.join(format!("{name}.ppm")));
        }
    }

    /// Keep frames coming while commands are pending.
    #[must_use]
    pub fn busy(&self) -> bool {
        true
    }
}
