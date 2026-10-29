//! Scripts that drive the gui and save screenshots, for looking at how the map behaves without
//! sitting in front of it. One command per line, `#` starts a comment:
//!
//! ```text
//! size 1280 800          # window inner size in physical pixels
//! dimension nether       # overworld, nether or end
//! samples 2              # samples per pixel and axis
//! panel off              # hide the settings window (on to show it again)
//! goto 1000 -500         # center the view on these block coordinates
//! zoom 6                 # log2 of quarts per screen point, 0 = 1 quart per point
//! zoom_to 12 2.5         # zoom there (around the center) over 2.5 seconds
//! pan_to 0 0 1           # move the center there (blocks) over 1 second
//! wait 0.5               # seconds
//! wait_idle              # until no tiles are being generated, prints how long that took
//! shot out/a.png         # save the next frame
//! record out/frames 4    # save every 4th frame as out/frames/00000.png, 00001.png, ...
//! stop                   # stop recording
//! quit
//! ```

use std::{collections::VecDeque, path::PathBuf, time::Instant};

use anyhow::{Context, bail};

use crate::dimension::Dimension;

pub enum Command {
    Size(u32, u32),
    Dimension(Dimension),
    Samples(u8),
    Panel(bool),
    Goto(f64, f64),
    Zoom(f64),
    ZoomTo(f64, f64),
    PanTo(f64, f64, f64),
    Wait(f64),
    WaitIdle,
    Shot(PathBuf),
    Record(PathBuf, u32),
    Stop,
    Quit,
}

impl Command {
    fn parse(line: &str) -> anyhow::Result<Option<Self>> {
        let line = line.split('#').next().unwrap().trim();
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else { return Ok(None) };
        let args: Vec<&str> = words.collect();
        let num = |i: usize| -> anyhow::Result<f64> {
            args.get(i).with_context(|| format!("{name}: missing argument {}", i + 1))?.parse::<f64>().context(name.to_string())
        };
        let path = |i: usize| -> anyhow::Result<PathBuf> {
            Ok(PathBuf::from(args.get(i).with_context(|| format!("{name}: missing path"))?))
        };
        Ok(Some(match name {
            "size" => Command::Size(num(0)? as u32, num(1)? as u32),
            "dimension" => Command::Dimension(
                args.first().and_then(|d| Dimension::from_name(d)).context("dimension: overworld, nether or end")?,
            ),
            "samples" => Command::Samples(num(0)? as u8),
            "panel" => Command::Panel(args.first() != Some(&"off")),
            "goto" => Command::Goto(num(0)?, num(1)?),
            "zoom" => Command::Zoom(num(0)?),
            "zoom_to" => Command::ZoomTo(num(0)?, num(1)?),
            "pan_to" => Command::PanTo(num(0)?, num(1)?, num(2)?),
            "wait" => Command::Wait(num(0)?),
            "wait_idle" => Command::WaitIdle,
            "shot" => Command::Shot(path(0)?),
            "record" => Command::Record(path(0)?, num(1).unwrap_or(1.0) as u32),
            "stop" => Command::Stop,
            "quit" => Command::Quit,
            _ => bail!("unknown command {name}"),
        }))
    }
}

/// What the script wants from the gui this frame.
#[derive(Default)]
pub struct Frame {
    /// settings to change before the frame, see `Command`
    pub commands: Vec<Command>,
    /// camera for this frame: center in quarts and zoom
    pub center: Option<[f64; 2]>,
    pub zoom: Option<f64>,
    /// save the frame there
    pub capture: Option<PathBuf>,
    pub quit: bool,
}

/// A command that takes more than one frame.
enum Running {
    Wait { until: Instant },
    WaitIdle,
    ZoomTo { from: f64, to: f64, start: Instant, secs: f64 },
    PanTo { from: [f64; 2], to: [f64; 2], start: Instant, secs: f64 },
}

pub struct Script {
    commands: VecDeque<Command>,
    running: Option<Running>,
    recording: Option<(PathBuf, u32)>,
    frame: u32,
    recorded: u32,
    started: Instant,
    idle_since: Option<Instant>,
}

impl Script {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut commands = VecDeque::new();
        for (i, line) in text.lines().enumerate() {
            if let Some(command) = Command::parse(line).with_context(|| format!("{}:{}", path.display(), i + 1))? {
                commands.push_back(command);
            }
        }
        let now = Instant::now();
        Ok(Self { commands, running: None, recording: None, frame: 0, recorded: 0, started: now, idle_since: None })
    }

    /// Advances the script by one frame. `camera` is the current center (quarts) and zoom,
    /// `busy` whether tiles are being generated.
    pub fn step(&mut self, camera: ([f64; 2], f64), busy: bool) -> Frame {
        let now = Instant::now();
        let mut frame = Frame::default();
        self.frame += 1;
        if let Some((dir, every)) = &self.recording
            && self.frame % every == 0
        {
            frame.capture = Some(dir.join(format!("{:05}.png", self.recorded)));
            self.recorded += 1;
        }

        loop {
            if let Some(running) = &self.running {
                let done = match *running {
                    Running::Wait { until } => now >= until,
                    Running::WaitIdle => {
                        // a frame without work could be a gap between tasks, so wait for a few
                        if busy {
                            self.idle_since = None;
                        }
                        let since = *self.idle_since.get_or_insert(now);
                        let idle = now.duration_since(since).as_secs_f64() > 0.3;
                        if idle {
                            println!("script: idle at {:.2}s", (since - self.started).as_secs_f64());
                        }
                        idle
                    }
                    Running::ZoomTo { from, to, start, secs } => {
                        let t = (now.duration_since(start).as_secs_f64() / secs).min(1.0);
                        frame.zoom = Some(from + (to - from) * t);
                        t >= 1.0
                    }
                    Running::PanTo { from, to, start, secs } => {
                        let t = (now.duration_since(start).as_secs_f64() / secs).min(1.0);
                        frame.center = Some([from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]);
                        t >= 1.0
                    }
                };
                if !done {
                    return frame;
                }
                self.running = None;
            }

            let Some(command) = self.commands.pop_front() else {
                // the script is done, but the window stays open unless it said quit
                return frame;
            };
            match command {
                Command::Goto(x, z) => frame.center = Some([x / 4.0, z / 4.0]),
                Command::Zoom(zoom) => frame.zoom = Some(zoom),
                Command::ZoomTo(to, secs) => {
                    let from = frame.zoom.unwrap_or(camera.1);
                    self.running = Some(Running::ZoomTo { from, to, start: now, secs });
                }
                Command::PanTo(x, z, secs) => {
                    let from = frame.center.unwrap_or(camera.0);
                    self.running = Some(Running::PanTo { from, to: [x / 4.0, z / 4.0], start: now, secs });
                }
                Command::Wait(secs) => {
                    self.running = Some(Running::Wait { until: now + std::time::Duration::from_secs_f64(secs) })
                }
                Command::WaitIdle => {
                    self.idle_since = None;
                    self.started = now;
                    self.running = Some(Running::WaitIdle);
                }
                Command::Shot(path) => {
                    frame.capture = Some(path);
                    // one shot per frame
                    return frame;
                }
                Command::Record(dir, every) => {
                    let _ = std::fs::create_dir_all(&dir);
                    self.recording = Some((dir, every.max(1)));
                    self.recorded = 0;
                }
                Command::Stop => self.recording = None,
                Command::Quit => {
                    frame.quit = true;
                    return frame;
                }
                command => frame.commands.push(command),
            }
        }
    }

    pub fn is_done(&self) -> bool {
        self.commands.is_empty() && self.running.is_none() && self.recording.is_none()
    }
}
