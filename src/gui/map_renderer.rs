use std::rc::Rc;

/// Background of the map where there are no tiles (yet), as sRGB.
const BACKGROUND_SRGB: f64 = 24.0 / 255.0;
/// rect (4 floats), uv (4 floats), alpha
const QUAD_FLOATS: usize = 9;

/// A tile uploaded to the GPU. The texture is freed when the last `Rc` is dropped.
pub struct TileTexture {
    bind_group: wgpu::BindGroup,
}

/// One textured rectangle of the map.
pub struct Quad {
    /// min x, min y, max x, max y in physical pixels
    pub rect: [f32; 4],
    /// min u, min v, max u, max v
    pub uv: [f32; 4],
    pub alpha: f32,
    pub tex: Rc<TileTexture>,
}

/// Draws the map tiles onto the surface.
pub struct MapRenderer {
    pipeline: wgpu::RenderPipeline,
    tile_layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    screen_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    /// the surface isn't sRGB, so the shader has to encode
    encode_srgb: bool,
    size: [u32; 2],
}

impl MapRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("map.wgsl"));

        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map screen"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map tile"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map tiles"),
            bind_group_layouts: &[Some(&screen_layout), Some(&tile_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map tiles"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_tile"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (QUAD_FLOATS * 4) as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_tile"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: filter,
                min_filter: filter,
                ..Default::default() // clamp to edge
            })
        };
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map screen"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            tile_layout,
            linear_sampler: sampler(wgpu::FilterMode::Linear),
            nearest_sampler: sampler(wgpu::FilterMode::Nearest),
            screen_buffer,
            screen_bind_group,
            instances: Self::instance_buffer(device, 256),
            encode_srgb: !surface_format.is_srgb(),
            size,
        }
    }

    fn instance_buffer(device: &wgpu::Device, quads: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map quads"),
            size: (quads * QUAD_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = [width, height];
    }

    /// Uploads a tile. `nearest` keeps single texels crisp when they are magnified.
    pub fn create_tile(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: u32,
        nearest: bool,
    ) -> TileTexture {
        let rgba: Vec<u8> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect();
        self.create_rgba_tile(device, queue, &rgba, size, nearest)
    }

    /// Uploads a tile with its own opacity, see `create_tile`.
    pub fn create_rgba_tile(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        size: u32,
        nearest: bool,
    ) -> TileTexture {
        let extent = wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map tile"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: None,
            },
            extent,
        );
        let view = texture.create_view(&Default::default());
        let sampler = if nearest {
            &self.nearest_sampler
        } else {
            &self.linear_sampler
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map tile"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        TileTexture { bind_group }
    }

    /// Clears `target` to the background and draws `quads` in order on it.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        quads: &[Quad],
    ) {
        let [w, h] = self.size;
        let mut screen: Vec<u8> = [w as f32, h as f32].iter().flat_map(|f| f.to_ne_bytes()).collect();
        screen.extend_from_slice(&(self.encode_srgb as u32).to_ne_bytes());
        screen.extend_from_slice(&0u32.to_ne_bytes());
        queue.write_buffer(&self.screen_buffer, 0, &screen);

        if !quads.is_empty() {
            let needed = (quads.len() * QUAD_FLOATS * 4) as u64;
            if self.instances.size() < needed {
                self.instances = Self::instance_buffer(device, quads.len().next_power_of_two());
            }
            let data: Vec<u8> = quads
                .iter()
                .flat_map(|q| q.rect.into_iter().chain(q.uv).chain([q.alpha]))
                .flat_map(f32::to_ne_bytes)
                .collect();
            queue.write_buffer(&self.instances, 0, &data);
        }

        // wgpu clear colors are linear for sRGB surfaces and written as is otherwise
        let background = if self.encode_srgb {
            BACKGROUND_SRGB
        } else {
            srgb_to_linear(BACKGROUND_SRGB)
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map tiles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: background,
                        g: background,
                        b: background,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.screen_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        for (i, quad) in quads.iter().enumerate() {
            let i = i as u32;
            pass.set_bind_group(1, &quad.tex.bind_group, &[]);
            pass.draw(0..4, i..i + 1);
        }
    }
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
