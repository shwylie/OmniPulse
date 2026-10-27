// OmniPulse's GPU visualiser: scenes made of GLSL passes (as Shadertoy's: buffers that read each other and
// their own last frame, an optional pass of many points, then the picture), run on wgpu with a device of
// their own. Each frame is read back as RGBA for the app to show, so any of Slint's renderers can show it.
// The same scenes' code runs on the Stream Widget in WebGL2: only the prelude (how textures and the
// uniforms are declared, which way up a texture is) differs.

pub mod audio;
pub mod scenes;

pub use audio::Audio;

/// Where a pass reads from: nothing, a pass of the scene (its picture this frame if it's already drawn,
/// else its last frame's, so a pass can read its own), or the picture the app gives (the song's cover)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input { None, Pass(usize), Art }

/// How big a pass's picture is: a share of the view's, or a size of its own (a swarm's state, one texel each)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size { Scaled(f32), Fixed(u32, u32) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// a buffer: mainImage over its whole picture, kept (half floats) for the passes after and its next frame
    Buffer,
    /// many points (this many), each placed by the scene's point(), drawn soft and added up
    Points(u32),
    /// the picture shown: mainImage, in the view's size
    Image,
}

#[derive(Clone, Debug)]
pub struct Pass {
    pub kind: Kind,
    pub size: Size,
    pub inputs: [Input; 4],
    pub src: &'static str,
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub id: &'static str,
    pub name: &'static str,
    pub passes: Vec<Pass>,
}

/// The GLSL every pass starts with, natively (naga's GLSL: Vulkan's way of declaring textures). A
/// texture's first row is its top, so ch*() and fetch*() turn it the other way up: the scenes' code sees
/// what GL does (y up, its first row the bottom), and runs as it is in WebGL too.
const PRELUDE: &str = r#"#version 450
layout(set = 0, binding = 0, std140) uniform U { vec4 res; vec4 tim; vec4 a0; vec4 a1; vec4 a2; vec4 chres[4]; } u;
layout(set = 0, binding = 1) uniform sampler S;
layout(set = 0, binding = 2) uniform texture2D T0;
layout(set = 0, binding = 3) uniform texture2D T1;
layout(set = 0, binding = 4) uniform texture2D T2;
layout(set = 0, binding = 5) uniform texture2D T3;
layout(set = 0, binding = 6) uniform texture2D TA;
#define iResolution vec3(u.res.xy, 1.0)
#define iTime u.tim.x
#define iFrame int(u.tim.y)
#define iTimeDelta u.tim.z
#define BASS u.a0.x
#define MID u.a0.y
#define HIGH u.a0.z
#define LEVEL u.a0.w
#define KICK u.a1.x
#define SNARE u.a1.y
#define HAT u.a1.z
#define BEAT u.a1.w
#define BUILD u.a2.x
#define DROP u.a2.y
#define WIDTH u.a2.z
#define HITS u.a2.w
vec4 ch0(vec2 uv) { return texture(sampler2D(T0, S), vec2(uv.x, 1.0 - uv.y)); }
vec4 ch1(vec2 uv) { return texture(sampler2D(T1, S), vec2(uv.x, 1.0 - uv.y)); }
vec4 ch2(vec2 uv) { return texture(sampler2D(T2, S), vec2(uv.x, 1.0 - uv.y)); }
vec4 ch3(vec2 uv) { return texture(sampler2D(T3, S), vec2(uv.x, 1.0 - uv.y)); }
vec4 fetch0(ivec2 p) { return texelFetch(sampler2D(T0, S), ivec2(p.x, int(u.chres[0].y) - 1 - p.y), 0); }
vec4 fetch1(ivec2 p) { return texelFetch(sampler2D(T1, S), ivec2(p.x, int(u.chres[1].y) - 1 - p.y), 0); }
vec2 chRes(int i) { return u.chres[i].xy; }
float spectrum(float x) { return textureLod(sampler2D(TA, S), vec2(clamp(x, 0.0, 1.0), 0.25), 0.0).r; }
vec2 wave(float x) { return textureLod(sampler2D(TA, S), vec2(clamp(x, 0.0, 1.0), 0.75), 0.0).rg * 2.0 - 1.0; }
"#;

const FRAG_MAIN: &str = r#"
layout(location = 0) out vec4 outColor;
void main() { vec4 c = vec4(0.0); mainImage(c, vec2(gl_FragCoord.x, u.res.y - gl_FragCoord.y)); outColor = c; }
"#;

const FULLSCREEN_VS: &str = r#"#version 450
void main() { vec2 p = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2)); gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }
"#;

const POINTS_VS_MAIN: &str = r#"
layout(location = 0) out vec4 vCol;
layout(location = 1) out vec2 vQ;
void main() {
    int vi = int(gl_VertexIndex);
    int id = vi / 6;
    int k = vi - id * 6;
    vec2 q = k == 0 ? vec2(-1.0, -1.0) : k == 1 ? vec2(1.0, -1.0) : k == 2 ? vec2(-1.0, 1.0) : k == 3 ? vec2(-1.0, 1.0) : k == 4 ? vec2(1.0, -1.0) : vec2(1.0, 1.0);
    vec4 col = vec4(0.0);
    float size = 1.0;
    vec4 p = point(id, col, size);
    gl_Position = p + vec4(q * size / u.res.xy * p.w, 0.0, 0.0);
    vCol = col;
    vQ = q;
}
"#;

const POINTS_FS: &str = r#"#version 450
layout(location = 0) in vec4 vCol;
layout(location = 1) in vec2 vQ;
layout(location = 0) out vec4 outColor;
void main() { float d = dot(vQ, vQ); if (d > 1.0) discard; float f = 1.0 - d; outColor = vCol * f * f; }
"#;

/// The prelude for the Stream Widget's WebGL2: the same names, GL's way
pub const PRELUDE_WEBGL: &str = r#"#version 300 es
precision highp float;
precision highp int;
layout(std140) uniform U { vec4 res; vec4 tim; vec4 a0; vec4 a1; vec4 a2; vec4 chres[4]; } u;
uniform sampler2D T0; uniform sampler2D T1; uniform sampler2D T2; uniform sampler2D T3; uniform sampler2D TA;
"#;

const BUF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// the audio texture: a row of the spectrum, a row of the left and right waves
pub const AUDIO_W: u32 = 256;

#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms { res: [f32; 4], tim: [f32; 4], a0: [f32; 4], a1: [f32; 4], a2: [f32; 4], chres: [[f32; 4]; 4] }

impl Uniforms {
    fn bytes(&self) -> &[u8] {
        // (plain f32s, laid out as std140's vec4s)
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, std::mem::size_of::<Self>()) }
    }
}

struct Target { tex: [wgpu::Texture; 2], view: [wgpu::TextureView; 2], w: u32, h: u32 }

struct Built {
    scene: Scene,
    pipes: Vec<wgpu::RenderPipeline>,
    uniforms: Vec<wgpu::Buffer>,
    /// each pass's pictures (two: this frame's and the last), for the view's size as it was
    targets: Vec<Target>,
    /// each pass's bindings, by which of its two pictures it draws into
    groups: Vec<[wgpu::BindGroup; 2]>,
    size: (u32, u32),
    readback: Option<(wgpu::Buffer, u32)>,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: String,
    layout: wgpu::BindGroupLayout,
    pipe_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    black: wgpu::TextureView,
    /// the picture given (the cover), and its size; None: black
    art: Option<(wgpu::TextureView, u32, u32)>,
    audio_tex: wgpu::Texture,
    audio_view: wgpu::TextureView,
    fullscreen: wgpu::ShaderModule,
    points_fs: wgpu::ShaderModule,
    built: Option<Built>,
    frame: u64,
    last_t: f32,
}

fn glsl(device: &wgpu::Device, label: &str, src: &str, stage: wgpu::naga::ShaderStage) -> Result<wgpu::ShaderModule, String> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let m = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Glsl { shader: src.into(), stage, defines: &[] } });
    match pollster::block_on(scope.pop()) { Some(e) => Err(format!("{label}: {e}")), None => Ok(m) }
}

impl Renderer {
    /// The GPU's (its best adapter: a discrete card over the built-in one), or why there's none
    pub fn new() -> Result<Renderer, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: false, compatible_surface: None, ..Default::default() }))
            .map_err(|e| format!("no GPU for the visualiser: {e}"))?;
        let name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("omnipulse-vis"), ..Default::default() })).map_err(|e| format!("the GPU wouldn't start: {e}"))?;
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vis"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                tex_entry(2), tex_entry(3), tex_entry(4), tex_entry(5), tex_entry(6),
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("vis"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("vis"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let tex = |w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| device.create_texture(&wgpu::TextureDescriptor { label: Some("vis"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
        let black = tex(1, 1, BUF_FORMAT, wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
        let audio_tex = tex(AUDIO_W, 2, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let audio_view = audio_tex.create_view(&Default::default());
        let fullscreen = glsl(&device, "fullscreen", FULLSCREEN_VS, wgpu::naga::ShaderStage::Vertex)?;
        let points_fs = glsl(&device, "points", POINTS_FS, wgpu::naga::ShaderStage::Fragment)?;
        Ok(Renderer { device, queue, adapter: name, layout, pipe_layout, sampler, black, audio_tex, audio_view, fullscreen, points_fs, built: None, frame: 0, last_t: 0.0, art: None })
    }

    pub fn adapter(&self) -> &str { &self.adapter }

    pub fn scene(&self) -> Option<&str> { self.built.as_ref().map(|b| b.scene.id) }

    /// The picture scenes can read (Input::Art): RGBA, top row first; None for none. The scene starts again
    /// from it (a new cover: its smoke made from the new one)
    pub fn set_art(&mut self, art: Option<(&[u8], u32, u32)>) {
        self.art = art.filter(|(px, w, h)| *w > 0 && *h > 0 && px.len() >= (*w * *h * 4) as usize).map(|(px, w, h)| {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some("vis art"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &px[..(w * h * 4) as usize],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            (tex.create_view(&Default::default()), w, h)
        });
        // (its bindings made again on the next frame)
        if let Some(b) = self.built.as_mut() { b.size = (0, 0); }
    }

    /// A scene, its passes' shaders made (a mistake in one: why, and the scene as it was)
    pub fn set_scene(&mut self, scene: &Scene) -> Result<(), String> {
        let mut pipes = Vec::new();
        for (i, p) in scene.passes.iter().enumerate() {
            let label = format!("{} pass {i}", scene.id);
            let blend_add = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } };
            let (vs, fs, blend) = match p.kind {
                Kind::Points(_) => (glsl(&self.device, &label, &format!("{PRELUDE}{}{POINTS_VS_MAIN}", p.src), wgpu::naga::ShaderStage::Vertex)?, self.points_fs.clone(), Some(blend_add)),
                _ => (self.fullscreen.clone(), glsl(&self.device, &label, &format!("{PRELUDE}{}{FRAG_MAIN}", p.src), wgpu::naga::ShaderStage::Fragment)?, None),
            };
            let format = if p.kind == Kind::Image { OUT_FORMAT } else { BUF_FORMAT };
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&label),
                layout: Some(&self.pipe_layout),
                vertex: wgpu::VertexState { module: &vs, entry_point: Some("main"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState { module: &fs, entry_point: Some("main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None,
                cache: None,
            });
            if let Some(e) = pollster::block_on(scope.pop()) { return Err(format!("{label}: {e}")); }
            pipes.push(pipe);
        }
        let uniforms = scene.passes.iter().map(|_| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("vis uniforms"), size: std::mem::size_of::<Uniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).collect();
        self.built = Some(Built { scene: scene.clone(), pipes, uniforms, targets: Vec::new(), groups: Vec::new(), size: (0, 0), readback: None });
        self.frame = 0;
        Ok(())
    }

    fn make_targets(&mut self, w: u32, h: u32) {
        let Some(b) = self.built.as_mut() else { return };
        let device = &self.device;
        b.targets = b.scene.passes.iter().map(|p| {
            let (tw, th) = match (p.kind, p.size) {
                (Kind::Image, _) => (w, h),
                (_, Size::Fixed(fw, fh)) => (fw.max(1), fh.max(1)),
                (_, Size::Scaled(s)) => (((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1)),
            };
            let format = if p.kind == Kind::Image { OUT_FORMAT } else { BUF_FORMAT };
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
            let one = || device.create_texture(&wgpu::TextureDescriptor { label: Some("vis pass"), size: wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let tex = [one(), one()];
            let view = [tex[0].create_view(&Default::default()), tex[1].create_view(&Default::default())];
            Target { tex, view, w: tw, h: th }
        }).collect();
        // (pass i, drawing into its picture `par`: an input drawn already this frame is read as it is now, a
        // later one (or itself) as it was last frame)
        let mut groups = Vec::new();
        for (i, p) in b.scene.passes.iter().enumerate() {
            let make = |par: usize| {
                let view_of = |inp: Input| -> &wgpu::TextureView {
                    match inp {
                        Input::Pass(j) if j < b.targets.len() && b.scene.passes[j].kind != Kind::Image => &b.targets[j].view[if j < i { par } else { 1 - par }],
                        Input::Art => self.art.as_ref().map(|a| &a.0).unwrap_or(&self.black),
                        _ => &self.black,
                    }
                };
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vis"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: b.uniforms[i].as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view_of(p.inputs[0])) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(view_of(p.inputs[1])) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(view_of(p.inputs[2])) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(view_of(p.inputs[3])) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.audio_view) },
                    ],
                })
            };
            groups.push([make(0), make(1)]);
        }
        b.groups = groups;
        b.size = (w, h);
        let row = (w * 4).div_ceil(256) * 256;
        b.readback = Some((device.create_buffer(&wgpu::BufferDescriptor { label: Some("vis readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }), row));
        self.frame = 0;
    }

    /// A frame, w × h, at `t` seconds, with the sound as it is: its pixels, RGBA, top row first
    pub fn render(&mut self, w: u32, h: u32, t: f32, audio: &Audio) -> Result<Vec<u8>, String> {
        let (w, h) = (w.clamp(16, 4096), h.clamp(16, 4096));
        if self.built.is_none() { return Err("no scene".into()); }
        if self.built.as_ref().map(|b| b.size) != Some((w, h)) { self.make_targets(w, h); }
        // (the sound: its spectrum and its waves, as the scenes read them)
        let mut px = vec![0u8; (AUDIO_W * 2 * 4) as usize];
        for x in 0..AUDIO_W as usize {
            let s = audio.spectrum_at(x as f32 / (AUDIO_W - 1) as f32);
            px[x * 4] = (s.clamp(0.0, 1.0) * 255.0) as u8;
            let (l, r) = audio.wave_at(x);
            let o = (AUDIO_W as usize + x) * 4;
            px[o] = ((l * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            px[o + 1] = ((r * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.audio_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(AUDIO_W * 4), rows_per_image: Some(2) },
            wgpu::Extent3d { width: AUDIO_W, height: 2, depth_or_array_layers: 1 },
        );
        let dt = if self.frame == 0 { 1.0 / 60.0 } else { (t - self.last_t).clamp(0.0, 0.25) };
        self.last_t = t;
        let par = (self.frame % 2) as usize;
        let b = self.built.as_ref().unwrap();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("vis") });
        let mut out = 0;
        for (i, p) in b.scene.passes.iter().enumerate() {
            let tg = &b.targets[i];
            let mut chres = [[0.0f32; 4]; 4];
            for (k, inp) in p.inputs.iter().enumerate() {
                match inp {
                    Input::Pass(j) => if let Some(t) = b.targets.get(*j) { chres[k] = [t.w as f32, t.h as f32, 0.0, 0.0]; },
                    // (the picture's size, and whether there is one)
                    Input::Art => chres[k] = self.art.as_ref().map(|a| [a.1 as f32, a.2 as f32, 1.0, 0.0]).unwrap_or([1.0, 1.0, 0.0, 0.0]),
                    Input::None => {}
                }
            }
            let u = Uniforms {
                res: [tg.w as f32, tg.h as f32, w as f32, h as f32],
                tim: [t, self.frame as f32, dt, 0.0],
                a0: [audio.bass, audio.mid, audio.high, audio.level],
                a1: [audio.kick, audio.snare, audio.hat, audio.beat as f32],
                a2: [audio.build, audio.drop, audio.width, audio.hits as f32],
                chres,
            };
            self.queue.write_buffer(&b.uniforms[i], 0, u.bytes());
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vis pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &tg.view[par], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&b.pipes[i]);
            rp.set_bind_group(0, &b.groups[i][par], &[]);
            match p.kind { Kind::Points(n) => rp.draw(0..n * 6, 0..1), _ => rp.draw(0..3, 0..1) }
            drop(rp);
            if p.kind == Kind::Image { out = i; }
        }
        let (buf, row) = b.readback.as_ref().unwrap();
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &b.targets[out].tex[par], mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("the GPU: {e}"))?;
        rx.recv().map_err(|_| "the GPU didn't answer".to_string())?.map_err(|e| format!("the GPU: {e}"))?;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        {
            let data = slice.get_mapped_range().map_err(|e| format!("the GPU: {e}"))?;
            for y in 0..h as usize { pixels.extend_from_slice(&data[y * *row as usize..y * *row as usize + w as usize * 4]); }
        }
        buf.unmap();
        self.frame += 1;
        Ok(pixels)
    }
}
