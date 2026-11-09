//! GPU renderer: one instanced-quad pipeline drawing solid rects and glyphs from an R8 atlas.
use crate::config::Rgb;
use crate::font::{self, CellMetrics, FontSet};
use std::collections::HashMap;
use std::sync::Arc;
use winit::window::Window;

const ATLAS: u32 = 2048;

const SHADER: &str = r#"
struct U { screen: vec2<f32>, atlas: vec2<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) color: vec4<f32>,
  @location(2) solid: f32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32,
      @location(0) pos: vec2<f32>, @location(1) size: vec2<f32>,
      @location(2) uv: vec4<f32>, @location(3) color: vec4<f32>) -> VOut {
  let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
  let p = pos + corner * size;
  var o: VOut;
  o.pos = vec4<f32>(p.x / u.screen.x * 2.0 - 1.0, 1.0 - p.y / u.screen.y * 2.0, 0.0, 1.0);
  o.uv = (uv.xy + corner * uv.zw) / u.atlas;
  o.color = color;
  o.solid = select(0.0, 1.0, uv.z <= 0.0);
  return o;
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
  let a = textureSampleLevel(tex, samp, i.uv, 0.0).r;
  if (i.solid > 0.5) { return i.color; }
  return vec4<f32>(i.color.rgb, i.color.a * a);
}
"#;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Inst {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 4],
    color: [f32; 4],
}

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Create GPU context compatible with the first window's surface.
    pub fn new(window: Arc<Window>) -> anyhow::Result<(Gpu, wgpu::Surface<'static>)> {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone())).with_env(),
        );
        let surface = instance.create_surface(window)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;
        log::info!("GPU adapter: {:?}", adapter.get_info().name);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("dtem"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))?;
        Ok((Gpu { instance, adapter, device, queue }, surface))
    }
}

#[derive(Clone, Copy)]
struct GlyphEntry {
    uv: [f32; 4],
    off_x: f32,
    off_y: f32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    atlas: wgpu::Texture,
    inst_buf: wgpu::Buffer,
    inst_cap: usize,
    glyphs: HashMap<(char, u8), Option<GlyphEntry>>,
    pack_x: u32,
    pack_y: u32,
    row_h: u32,
    bg: Vec<Inst>,
    fg: Vec<Inst>,
    pub metrics: CellMetrics,
    pub atlas_full: bool,
}

pub fn rgba(c: Rgb, a: f32) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, a]
}

impl Renderer {
    pub fn new(gpu: &Gpu, surface: wgpu::Surface<'static>, width: u32, height: u32, metrics: CellMetrics) -> Renderer {
        let device = &gpu.device;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dtem shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dtem pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Inst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
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
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let inst_cap = 4096;
        let inst_buf = Self::make_buf(device, inst_cap);
        let mut r = Renderer {
            surface,
            config,
            pipeline,
            bind_group,
            uniform,
            atlas,
            inst_buf,
            inst_cap,
            glyphs: HashMap::new(),
            pack_x: 0,
            pack_y: 0,
            row_h: 0,
            bg: Vec::new(),
            fg: Vec::new(),
            metrics,
            atlas_full: false,
        };
        r.reset_atlas(gpu);
        r
    }

    fn make_buf(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (cap * std::mem::size_of::<Inst>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn reset_atlas(&mut self, gpu: &Gpu) {
        self.glyphs.clear();
        // Reserve a 2x2 white block at origin (unused, solid quads don't sample it)
        self.pack_x = 4;
        self.pack_y = 0;
        self.row_h = 4;
        let white = [255u8; 16];
        self.upload(gpu, 0, 0, 4, 4, &white);
    }

    fn upload(&self, gpu: &Gpu, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    pub fn set_metrics(&mut self, gpu: &Gpu, m: CellMetrics) {
        self.metrics = m;
        self.reset_atlas(gpu);
    }

    pub fn resize(&mut self, gpu: &Gpu, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn size(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.bg.push(Inst { pos: [x.round(), y.round()], size: [w.round(), h.round()], uv: [0.0; 4], color });
    }

    /// A rect drawn above text (cursor, underline).
    pub fn rect_top(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.fg.push(Inst { pos: [x.round(), y.round()], size: [w.round(), h.round()], uv: [0.0; 4], color });
    }

    fn glyph_entry(&mut self, gpu: &Gpu, fonts: &mut FontSet, c: char, style: u8) -> Option<GlyphEntry> {
        if let Some(e) = self.glyphs.get(&(c, style)) {
            return *e;
        }
        let entry = (|| {
            let font = fonts.font_for(c, style)?;
            let (m, bitmap) = font::rasterize(&font, c, self.metrics.px);
            if m.width == 0 || m.height == 0 {
                return None;
            }
            let (w, h) = (m.width as u32, m.height as u32);
            if self.pack_x + w + 1 > ATLAS {
                self.pack_x = 0;
                self.pack_y += self.row_h + 1;
                self.row_h = 0;
            }
            if self.pack_y + h + 1 > ATLAS {
                self.atlas_full = true;
                return None;
            }
            let (x, y) = (self.pack_x, self.pack_y);
            self.upload(gpu, x, y, w, h, &bitmap);
            self.pack_x += w + 1;
            self.row_h = self.row_h.max(h);
            Some(GlyphEntry {
                uv: [x as f32, y as f32, w as f32, h as f32],
                off_x: m.xmin as f32,
                off_y: -(m.height as f32 + m.ymin as f32),
            })
        })();
        if self.atlas_full {
            return entry;
        }
        self.glyphs.insert((c, style), entry);
        entry
    }

    /// Draw a character whose cell's top-left is (x, y).
    pub fn glyph(&mut self, gpu: &Gpu, fonts: &mut FontSet, c: char, style: u8, x: f32, y: f32, color: [f32; 4]) {
        if c == ' ' || c == '\0' {
            return;
        }
        if let Some(g) = self.glyph_entry(gpu, fonts, c, style) {
            let gx = (x + g.off_x).round();
            let gy = (y + self.metrics.ascent + g.off_y).round();
            self.fg.push(Inst { pos: [gx, gy], size: [g.uv[2], g.uv[3]], uv: g.uv, color });
        }
    }

    pub fn text(&mut self, gpu: &Gpu, fonts: &mut FontSet, s: &str, x: f32, y: f32, max_w: f32, color: [f32; 4]) {
        let mut cx = x;
        for c in s.chars() {
            if cx + self.metrics.width > x + max_w {
                break;
            }
            self.glyph(gpu, fonts, c, font::REGULAR, cx, y, color);
            cx += self.metrics.width;
        }
    }

    /// Submit the frame. Returns false if the surface needs reconfiguration.
    pub fn present(&mut self, gpu: &Gpu, clear: Rgb) {
        if self.atlas_full {
            // Atlas overflow: start over next frame.
            self.atlas_full = false;
            self.reset_atlas(gpu);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                self.bg.clear();
                self.fg.clear();
                return;
            }
            _ => {
                self.bg.clear();
                self.fg.clear();
                return;
            }
        };
        let mut all = std::mem::take(&mut self.bg);
        all.append(&mut self.fg);
        if all.len() > self.inst_cap {
            self.inst_cap = all.len().next_power_of_two();
            self.inst_buf = Self::make_buf(&gpu.device, self.inst_cap);
        }
        if !all.is_empty() {
            gpu.queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&all));
        }
        let u = [self.config.width as f32, self.config.height as f32, ATLAS as f32, ATLAS as f32];
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&u));

        let view = frame.texture.create_view(&Default::default());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let c = rgba(clear, 1.0);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !all.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.inst_buf.slice(..));
                pass.draw(0..4, 0..all.len() as u32);
            }
        }
        gpu.queue.submit(Some(enc.finish()));
        gpu.queue.present(frame);
        all.clear();
        self.bg = all;
    }
}
