//! Saving what is on the screen as a png, for scripts.

/// A frame copied into a buffer the cpu can read.
pub struct Readback {
    buffer: wgpu::Buffer,
    size: [u32; 2],
    bytes_per_row: u32,
}

impl Readback {
    pub fn copy(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> Self {
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
    pub fn save(
        self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        path: std::path::PathBuf,
    ) -> anyhow::Result<()> {
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
