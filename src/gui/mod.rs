mod map_renderer;
mod map_view;
mod script;
mod tiles;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use steel_worldgen::biomes::BiomeSourceKind;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

use crate::{
    dimension::Dimension,
    gui::{
        map_renderer::{MapRenderer, Quad},
        map_view::{MapSettings, MapView},
        script::{Command, Script},
        tiles::TileCache,
    },
};

struct App {
    seed: u64,
    dimension: Dimension,
    script: Option<Script>,
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let window_attributes = Window::default_attributes().with_title("cubox");
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
        self.state = Some(pollster::block_on(State::new(window, self.seed, self.dimension)).unwrap())
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let state = match &mut self.state {
            Some(state) => state,
            None => return,
        };

        let response = state.egui_state.on_window_event(&state.window, &event);
        if response.repaint {
            state.window.request_redraw();
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                let capture = match &mut self.script {
                    Some(script) => {
                        let frame = script.step(
                            (state.ui.map.camera.center, state.ui.map.camera.zoom),
                            state.ui.tiles.pending() > 0,
                        );
                        if frame.quit {
                            event_loop.exit();
                            return;
                        }
                        state.apply_script(frame.commands, frame.center, frame.zoom);
                        // keep going until the script is done, it doesn't send input that would wake us
                        if !script.is_done() {
                            state.window.request_redraw();
                        }
                        frame.capture
                    }
                    None => None,
                };
                state.update();
                match state.render(capture.as_deref()) {
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("{e}");
                        event_loop.exit();
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key_code),
                        state: key_state,
                        ..
                    },
                ..
            } if !response.consumed => state.handle_key(event_loop, key_code, key_state.is_pressed()),
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // drop the window and egui's clipboard while the wayland connection still exists,
        // after the event loop is gone the clipboard thread segfaults tearing itself down
        self.state = None;
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // only redraw when something changes instead of spinning at the refresh rate
        let Some(state) = &mut self.state else { return };
        match state.repaint_at {
            Some(at) if at <= Instant::now() => {
                state.repaint_at = None;
                state.window.request_redraw();
            }
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

struct State {
    window: Arc<Window>,
    device: wgpu::Device,

    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    is_surface_configured: bool,

    queue: wgpu::Queue,

    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    map_renderer: MapRenderer,
    /// when egui wants the next frame (e.g. for a blinking text cursor)
    repaint_at: Option<Instant>,

    ui: Ui,
}

/// Everything that is shown in the window.
struct Ui {
    seed: u64,
    seed_text: String,
    dimension: Dimension,
    sample_height: i32,
    samples: u8,
    biome_source: Arc<BiomeSourceKind>,
    map: MapView,
    tiles: TileCache,
    /// whether the settings window is shown
    panel: bool,
    /// block coordinates for "Go"
    goto: [i64; 2],
    /// biome under the cursor, cached for the quart it was sampled at
    hovered_biome: Option<([i32; 2], String)>,
    /// what the map view wants drawn this frame
    quads: Vec<Quad>,
}

impl State {
    pub async fn new(window: Arc<Window>, seed: u64, dimension: Dimension) -> anyhow::Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,

            // default stuff
            flags: Default::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });
        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: Default::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: true,
            })
            .await?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: Default::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        // egui does its own gamma handling and expects a non sRGB framebuffer, preferably 8 bit
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|c| matches!(c, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| surface_caps.formats.iter().find(|c| !c.is_srgb()))
            .copied()
            .unwrap_or(surface_caps.formats[0]);
        log::info!("surface format {surface_format:?}, offered {:?}", surface_caps.formats);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            // vsync. without it frames are drawn as fast as possible while tiles load, taking cpu from the workers
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, Default::default());
        let map_renderer = MapRenderer::new(&device, surface_format, [size.width, size.height]);

        let biome_source = Arc::new(dimension.biome_source(seed));
        let sample_height = dimension.default_height();
        let samples = 2;
        let ui = Ui {
            seed,
            seed_text: seed.to_string(),
            dimension,
            sample_height,
            samples,
            tiles: TileCache::new(biome_source.clone(), sample_height, samples),
            biome_source,
            map: MapView::new(),
            panel: true,
            goto: [0, 0],
            hovered_biome: None,
            quads: Vec::new(),
        };

        Ok(Self {
            window,
            surface,
            device,
            queue,
            surface_config,
            is_surface_configured: false,
            egui_ctx,
            egui_state,
            egui_renderer,
            map_renderer,
            repaint_at: None,
            ui,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
            self.map_renderer.resize(width, height);
            self.is_surface_configured = true;
        }
    }

    /// Applies what a script wants for this frame.
    fn apply_script(&mut self, commands: Vec<Command>, center: Option<[f64; 2]>, zoom: Option<f64>) {
        let mut resized = None;
        let ui = &mut self.ui;
        for command in commands {
            match command {
                Command::Size(w, h) => {
                    // when the size is applied right away there is no resized event
                    if let Some(size) = self.window.request_inner_size(PhysicalSize::new(w, h)) {
                        resized = Some(size);
                    }
                }
                Command::Dimension(dimension) => ui.set_dimension(dimension),
                Command::Samples(samples) => {
                    ui.samples = samples;
                    ui.reset_tiles();
                }
                Command::Panel(on) => ui.panel = on,
                _ => {}
            }
        }
        if let Some(center) = center {
            ui.map.camera.center = center;
        }
        if let Some(zoom) = zoom {
            ui.map.camera.zoom = zoom;
        }
        if let Some(size) = resized {
            self.resize(size.width, size.height);
        }
    }

    /// Renders a frame, and if `capture` is given also saves it there as png.
    pub fn render(&mut self, capture: Option<&Path>) -> anyhow::Result<()> {
        if !self.is_surface_configured {
            return Ok(());
        }

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let ui = &mut self.ui;
        let full_output = self.egui_ctx.run_ui(raw_input, |root| ui.show(root));
        self.egui_state
            .handle_platform_output(&self.window, full_output.platform_output);

        // keep drawing while tiles are generating, otherwise only when egui asks for it
        let repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay);
        if self.ui.tiles.pending() > 0 || repaint_delay.is_zero() {
            self.window.request_redraw();
        } else {
            self.repaint_at = Instant::now().checked_add(repaint_delay);
        }

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                // Skip this frame
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                // You could recreate the devices and all resources
                // created with it here, but we'll just bail
                anyhow::bail!("Lost device");
            }
        };
        let view = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Cubox Render Encoder"),
        });

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.surface_config.width, self.surface_config.height],
            pixels_per_point: full_output.pixels_per_point,
        };
        let paint_jobs = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        for (id, deltas) in &full_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let egui_cmds =
            self.egui_renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, &paint_jobs, &screen);

        self.draw(&mut encoder, &view, &paint_jobs, &screen);
        // the surface can't be read back, so a captured frame is drawn a second time into a texture that can
        let capturable = matches!(
            self.surface_config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
        );
        if capture.is_some() && !capturable {
            anyhow::bail!("can only capture 8 bit surfaces, not {:?}", self.surface_config.format);
        }
        let readback = capture.map(|_| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("capture"),
                size: wgpu::Extent3d {
                    width: screen.size_in_pixels[0],
                    height: screen.size_in_pixels[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.draw(
                &mut encoder,
                &texture.create_view(&Default::default()),
                &paint_jobs,
                &screen,
            );
            Readback::copy(&self.device, &mut encoder, &texture)
        });

        self.queue
            .submit(egui_cmds.into_iter().chain(std::iter::once(encoder.finish())));
        if let (Some(readback), Some(path)) = (readback, capture) {
            readback.save(&self.device, self.surface_config.format, path.to_owned())?;
        }
        self.queue.present(output);

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        Ok(())
    }

    /// Draws the map and egui on top into `target`.
    fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        paint_jobs: &[egui::ClippedPrimitive],
        screen: &egui_wgpu::ScreenDescriptor,
    ) {
        // the map covers the whole window, egui goes on top
        self.map_renderer
            .render(&self.device, &self.queue, encoder, target, &self.ui.quads);

        let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Cubox Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut render_pass = render_pass.forget_lifetime();
        self.egui_renderer.render(&mut render_pass, paint_jobs, screen);
    }

    fn handle_key(&self, event_loop: &ActiveEventLoop, code: KeyCode, is_pressed: bool) {
        if let (KeyCode::Escape, true) = (code, is_pressed) {
            event_loop.exit()
        }
    }

    fn update(&mut self) {
        let now = self.egui_ctx.input(|i| i.time);
        if self.ui.tiles.poll(now, &self.device, &self.queue, &self.map_renderer) {
            self.window.request_redraw();
        }
    }
}

/// A frame copied into a buffer the cpu can read.
struct Readback {
    buffer: wgpu::Buffer,
    size: [u32; 2],
    bytes_per_row: u32,
}

impl Readback {
    fn copy(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> Self {
        let size = [texture.width(), texture.height()];
        // rows of a texture copy have to be aligned to 256 bytes
        let bytes_per_row = (size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: (bytes_per_row * size[1]) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        Self {
            buffer,
            size,
            bytes_per_row,
        }
    }

    /// Waits for the copy and writes a png in the background.
    fn save(self, device: &wgpu::Device, format: wgpu::TextureFormat, path: PathBuf) -> anyhow::Result<()> {
        self.buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let [w, h] = self.size;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        {
            let data = self
                .buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            for row in data.chunks_exact(self.bytes_per_row as usize) {
                for px in row[..(w * 4) as usize].as_chunks::<4>().0 {
                    rgb.extend_from_slice(&if bgra {
                        [px[2], px[1], px[0]]
                    } else {
                        [px[0], px[1], px[2]]
                    });
                }
            }
        }
        std::thread::spawn(move || {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = image::RgbImage::from_raw(w, h, rgb).unwrap().save(&path) {
                log::error!("saving {}: {e}", path.display());
            }
        });
        Ok(())
    }
}

impl Ui {
    fn set_dimension(&mut self, dimension: Dimension) {
        self.dimension = dimension;
        self.sample_height = dimension.default_height();
        self.biome_source = Arc::new(dimension.biome_source(self.seed));
        self.reset_tiles();
    }

    /// Throws away all tiles after the world or how it is sampled changed.
    fn reset_tiles(&mut self) {
        self.hovered_biome = None;
        self.tiles
            .clear(self.biome_source.clone(), self.sample_height, self.samples);
    }

    fn show(&mut self, root: &mut egui::Ui) {
        let settings = MapSettings {
            sample_height: self.sample_height,
            samples: self.samples,
        };
        self.quads.clear();
        egui::CentralPanel::no_frame().show(root, |ui| self.map.show(ui, &mut self.tiles, settings, &mut self.quads));
        if !self.panel {
            return;
        }

        let level = self.map.level(root.pixels_per_point());
        let mut changed = false;
        let mut new_dimension = None;
        let mut go = false;
        egui::Window::new("cubox").resizable(false).show(root.ctx(), |ui| {
            egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
                ui.label("Seed");
                if ui.text_edit_singleline(&mut self.seed_text).changed()
                    && let Ok(seed) = self.seed_text.trim().parse::<u64>()
                    && seed != self.seed
                {
                    self.seed = seed;
                    self.biome_source = Arc::new(self.dimension.biome_source(seed));
                    changed = true;
                }
                ui.end_row();

                ui.label("Dimension");
                egui::ComboBox::from_id_salt("dimension").selected_text(self.dimension.name()).show_ui(ui, |ui| {
                    for dimension in Dimension::ALL {
                        if ui.selectable_label(dimension == self.dimension, dimension.name()).clicked() && dimension != self.dimension {
                            new_dimension = Some(dimension);
                        }
                    }
                });
                ui.end_row();

                ui.label("Height (y)");
                ui.add_enabled_ui(self.dimension != Dimension::End, |ui| {
                    let range = if self.dimension == Dimension::Nether { 0..=128 } else { -64..=320 };
                    changed |= ui.add(egui::Slider::new(&mut self.sample_height, range)).changed();
                });
                ui.end_row();

                ui.label("Samples / pixel")
                    .on_hover_text("Biome samples averaged per pixel and axis. Higher shows more of the real detail far out, but is slower");
                ui.horizontal(|ui| {
                    for n in [1u8, 2, 4, 8] {
                        changed |= ui.selectable_value(&mut self.samples, n, format!("{}", n as u32 * n as u32)).changed();
                    }
                });
                ui.end_row();

                ui.label("Go to (x z)");
                ui.horizontal(|ui| {
                    for v in &mut self.goto {
                        let response = ui.add(egui::DragValue::new(v).speed(16.0));
                        go |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    }
                    go |= ui.button("Go").clicked();
                });
                ui.end_row();
            });
            ui.separator();

            let blocks_per_px = 4u64 << level;
            ui.label(format!("Level {level}: 1 tile pixel = {blocks_per_px}x{blocks_per_px} blocks"));
            if let Some(q) = self.map.hovered_quart {
                if self.hovered_biome.as_ref().is_none_or(|(at, _)| *at != q) {
                    let biome = self.biome_source.chunk_sampler().sample(q[0], self.sample_height >> 2, q[1]);
                    self.hovered_biome = Some((q, biome.key.path.to_string()));
                }
                let name = &self.hovered_biome.as_ref().unwrap().1;
                ui.label(format!("x {} z {}: {name}", q[0] * 4, q[1] * 4));
            } else {
                ui.label("");
            }
            ui.label(format!("Tiles generating: {}", self.tiles.pending()));
        });

        if go {
            self.map.camera.center = [self.goto[0] as f64 / 4.0, self.goto[1] as f64 / 4.0];
        }
        if let Some(dimension) = new_dimension {
            self.set_dimension(dimension);
        } else if changed {
            self.reset_tiles();
        }
        if go || changed || new_dimension.is_some() {
            root.ctx().request_repaint();
        }
    }
}

pub fn run(seed: u64, dimension: Dimension, script: Option<PathBuf>) -> anyhow::Result<()> {
    env_logger::init();
    let script = script.map(|path| Script::load(&path)).transpose()?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        seed,
        dimension,
        script,
        state: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
