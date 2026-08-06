use vello_common::TextureId;
use vello_common::geometry::RectU16;
use vello_common::kurbo::{Affine, Rect};
use vello_common::paint::{Image, ImageId, ImageSource};
use vello_common::peniko::{Extend, ImageQuality, ImageSampler, color::PremulRgba8};
use vello_common::pixmap::Pixmap;
use vello_hybrid::{
    RenderSettings, RenderSize, SampleRect, Scene, WebGlRenderer, WebGlTextureBindings,
};
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlTexture};

const FALLBACK_VIEWPORT_SIZE: u16 = 1024;
const ANIMATION_SPEED: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    ImagePaint,
    ExternalTexture,
}

impl RenderMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ImagePaint => "image paint",
            Self::ExternalTexture => "external texture",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::ImagePaint => Self::ExternalTexture,
            Self::ExternalTexture => Self::ImagePaint,
        }
    }
}

struct PreparedTextures {
    size: u16,
    external: Vec<WebGlTexture>,
    atlas: Vec<ImageId>,
    bindings: WebGlTextureBindings,
}

impl PreparedTextures {
    fn new(size: u16) -> Self {
        Self {
            size,
            external: Vec::new(),
            atlas: Vec::new(),
            bindings: WebGlTextureBindings::new(),
        }
    }

    fn len(&self) -> usize {
        self.external.len()
    }
}

#[derive(Debug)]
struct AnimatedRect {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
}

pub struct BenchRenderer {
    canvas: HtmlCanvasElement,
    gl: Gl,
    renderer: WebGlRenderer,
    resources: vello_hybrid::Resources,
    image_scene: Scene,
    external_scene: Scene,
    textures: Option<PreparedTextures>,
    rects: Vec<AnimatedRect>,
    rng: Rng,
    active_count: usize,
    draw_size: u16,
    last_animation_time: f64,
    viewport_width: u16,
    viewport_height: u16,
    render_size: RenderSize,
}

impl BenchRenderer {
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Self, String> {
        let (viewport_width, viewport_height) = viewport_size();
        canvas.set_width(u32::from(viewport_width));
        canvas.set_height(u32::from(viewport_height));

        let gl: Gl = canvas
            .get_context("webgl2")
            .map_err(|_| "failed to query WebGL2 context".to_string())?
            .ok_or_else(|| "WebGL2 is unavailable on this device".to_string())?
            .dyn_into()
            .map_err(|_| "canvas context was not WebGL2".to_string())?;

        let (renderer, resources) = WebGlRenderer::new_with(canvas, RenderSettings::default());

        Ok(Self {
            canvas: canvas.clone(),
            gl,
            renderer,
            resources,
            image_scene: Scene::new(viewport_width, viewport_height),
            external_scene: Scene::new(viewport_width, viewport_height),
            textures: None,
            rects: Vec::new(),
            rng: Rng::new(0xDEAD_BEEF),
            active_count: 0,
            draw_size: 4,
            last_animation_time: 0.0,
            viewport_width,
            viewport_height,
            render_size: RenderSize {
                width: u32::from(viewport_width),
                height: u32::from(viewport_height),
            },
        })
    }

    pub fn viewport_dimensions(&self) -> (u16, u16) {
        (self.viewport_width, self.viewport_height)
    }

    pub fn max_texture_size(&self) -> u32 {
        self.gl
            .get_parameter(Gl::MAX_TEXTURE_SIZE)
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0) as u32
    }

    pub fn begin_texture_set(&mut self, size: u16) {
        self.delete_textures();
        self.textures = Some(PreparedTextures::new(size));
    }

    pub fn prepared_texture_count(&self) -> usize {
        self.textures.as_ref().map_or(0, PreparedTextures::len)
    }

    /// Generate one of the same deterministic radial-wave images used by vello_bench2's
    /// image-paint rectangle benchmark, then upload identical premultiplied pixels to both the
    /// Vello image atlas and a standalone WebGL texture. This happens outside measured phases.
    pub fn prepare_next_texture(&mut self) -> Result<(), String> {
        let prepared = self
            .textures
            .as_mut()
            .ok_or_else(|| "texture set was not initialized".to_string())?;
        let texture_index = prepared.external.len();
        let size = prepared.size;
        let pixmap = make_image_pixmap(size, texture_index);

        let atlas_id = self.renderer.upload_image(&mut self.resources, &pixmap);
        let texture = self
            .gl
            .create_texture()
            .ok_or_else(|| "WebGL failed to create a texture".to_string())?;
        self.gl.active_texture(Gl::TEXTURE0);
        self.gl.bind_texture(Gl::TEXTURE_2D, Some(&texture));
        self.gl
            .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::NEAREST as i32);
        self.gl
            .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::NEAREST as i32);
        self.gl
            .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
        self.gl
            .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
        self.gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 1);
        self.gl
            .tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                Gl::TEXTURE_2D,
                0,
                Gl::RGBA8 as i32,
                i32::from(size),
                i32::from(size),
                0,
                Gl::RGBA,
                Gl::UNSIGNED_BYTE,
                Some(pixmap.data_as_u8_slice()),
            )
            .map_err(|_| format!("failed to upload {size}x{size} texture {texture_index}"))?;

        let texture_id = TextureId(texture_index as u64 + 1);
        prepared.bindings.insert(texture_id, texture.clone());
        prepared.external.push(texture);
        prepared.atlas.push(atlas_id);
        self.gl.bind_texture(Gl::TEXTURE_2D, None);
        Ok(())
    }

    pub fn configure_scene(&mut self, image_count: usize, draw_size: u16) -> Result<(), String> {
        let prepared = self
            .textures
            .as_ref()
            .ok_or_else(|| "textures have not been prepared".to_string())?;
        if prepared.len() < 1 {
            return Err("at least one texture must be prepared".to_string());
        }

        while self.rects.len() < image_count {
            self.rects.push(random_rect(
                &mut self.rng,
                self.viewport_width,
                self.viewport_height,
            ));
        }
        self.active_count = image_count;
        self.draw_size = draw_size.max(1);
        self.last_animation_time = 0.0;
        Ok(())
    }

    /// Animate, record, and submit one strategy. Scene recording mirrors vello_bench2's animated
    /// benchmark loop and is included in the measured frame interval.
    pub fn render_once(&mut self, mode: RenderMode, now: f64) -> Result<(), String> {
        if self.resize_to_viewport() {
            self.last_animation_time = now;
        }
        self.animate(now);
        self.record_scene(mode)?;
        let bindings = &self
            .textures
            .as_ref()
            .ok_or_else(|| "textures have not been prepared".to_string())?
            .bindings;
        let scene = match mode {
            RenderMode::ImagePaint => &self.image_scene,
            RenderMode::ExternalTexture => &self.external_scene,
        };
        self.renderer
            .render(scene, &mut self.resources, &self.render_size, bindings)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn animate(&mut self, now: f64) {
        let dt = ((now - self.last_animation_time) / 1000.0).clamp(0.0, 0.1) * ANIMATION_SPEED;
        self.last_animation_time = now;
        let max_x = f64::from(self.viewport_width.saturating_sub(self.draw_size));
        let max_y = f64::from(self.viewport_height.saturating_sub(self.draw_size));
        for rect in self.rects.iter_mut().take(self.active_count) {
            rect.x += rect.vx * dt;
            rect.y += rect.vy * dt;
            bounce(&mut rect.x, &mut rect.vx, max_x);
            bounce(&mut rect.y, &mut rect.vy, max_y);
        }
    }

    fn resize_to_viewport(&mut self) -> bool {
        let (width, height) = viewport_size();
        if width == self.viewport_width && height == self.viewport_height {
            return false;
        }

        self.canvas.set_width(u32::from(width));
        self.canvas.set_height(u32::from(height));
        self.image_scene = Scene::new(width, height);
        self.external_scene = Scene::new(width, height);
        self.viewport_width = width;
        self.viewport_height = height;
        self.render_size = RenderSize {
            width: u32::from(width),
            height: u32::from(height),
        };
        true
    }

    fn record_scene(&mut self, mode: RenderMode) -> Result<(), String> {
        let prepared = self
            .textures
            .as_ref()
            .ok_or_else(|| "textures have not been prepared".to_string())?;
        let texture_size = prepared.size;
        let pool_size = prepared.len();
        let source_region = RectU16::new(0, 0, texture_size, texture_size);
        let scale = f64::from(self.draw_size) / f64::from(texture_size);

        match mode {
            RenderMode::ExternalTexture => {
                self.external_scene.reset();
                for (index, rect) in self.rects.iter().take(self.active_count).enumerate() {
                    let transform = Affine::translate((rect.x, rect.y)) * Affine::scale(scale);
                    self.external_scene.draw_texture_rects(
                        TextureId((index % pool_size) as u64 + 1),
                        ImageQuality::Low,
                        [SampleRect {
                            source_region,
                            transform,
                        }],
                    );
                }
            }
            RenderMode::ImagePaint => {
                self.image_scene.reset();
                for (index, rect) in self.rects.iter().take(self.active_count).enumerate() {
                    let image = Image {
                        image: ImageSource::opaque_id_with_transparency_hint(
                            prepared.atlas[index % pool_size],
                            true,
                        ),
                        sampler: ImageSampler {
                            x_extend: Extend::Pad,
                            y_extend: Extend::Pad,
                            quality: ImageQuality::Low,
                            alpha: 1.0,
                        },
                    };
                    self.image_scene.set_paint_transform(
                        Affine::translate((rect.x, rect.y)) * Affine::scale(scale),
                    );
                    self.image_scene.set_paint(image);
                    self.image_scene.fill_rect(&Rect::new(
                        rect.x,
                        rect.y,
                        rect.x + f64::from(self.draw_size),
                        rect.y + f64::from(self.draw_size),
                    ));
                    self.image_scene.reset_paint_transform();
                }
            }
        }
        Ok(())
    }

    pub fn delete_textures(&mut self) {
        if let Some(prepared) = self.textures.take() {
            for texture in prepared.external {
                self.gl.delete_texture(Some(&texture));
            }
            for image_id in prepared.atlas {
                self.renderer.destroy_image(&mut self.resources, image_id);
            }
        }
    }
}

impl Drop for BenchRenderer {
    fn drop(&mut self) {
        self.delete_textures();
    }
}

fn random_rect(rng: &mut Rng, viewport_width: u16, viewport_height: u16) -> AnimatedRect {
    AnimatedRect {
        x: rng.f64() * f64::from(viewport_width),
        y: rng.f64() * f64::from(viewport_height),
        vx: (rng.f64() - 0.5) * 200.0,
        vy: (rng.f64() - 0.5) * 200.0,
    }
}

fn viewport_size() -> (u16, u16) {
    let window = web_sys::window();
    let dimension = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
        value
            .ok()
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map_or(FALLBACK_VIEWPORT_SIZE, |value| {
                value.min(f64::from(u16::MAX)).round() as u16
            })
    };
    window.map_or((FALLBACK_VIEWPORT_SIZE, FALLBACK_VIEWPORT_SIZE), |window| {
        (
            dimension(window.inner_width()),
            dimension(window.inner_height()),
        )
    })
}

/// Pixel-for-pixel equivalent to vello_bench2's image-paint generator at its native 64px size;
/// the same formula is generalized to the configured allocation size.
fn make_image_pixmap(size: u16, image_index: usize) -> Pixmap {
    let mut rng = Rng::new(0xCAFE_BABE ^ ((image_index as u64 + 1) * 0x9E37_79B9));
    let side = f64::from(size);
    let center = side / 2.0;
    let c1 = rng.color();
    let c2 = rng.color();
    let freq = rng.f64() * 5.0 + 3.0;
    let max_dist = (center * center * 2.0).sqrt();
    let mut pixels = Vec::with_capacity(usize::from(size) * usize::from(size));

    for y in 0..size {
        for x in 0..size {
            let dx = f64::from(x) - center;
            let dy = f64::from(y) - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let wave = (dist * freq * std::f64::consts::TAU / side).sin();
            let mix = (wave * wave) as f32;
            let alpha = 1.0 - 0.7 * (dist / max_dist) as f32;
            let lerp = |a: u8, b: u8| {
                let component = f32::from(a) + (f32::from(b) - f32::from(a)) * mix;
                (component * alpha) as u8
            };
            pixels.push(PremulRgba8 {
                r: lerp(c1[0], c2[0]),
                g: lerp(c1[1], c2[1]),
                b: lerp(c1[2], c2[2]),
                a: (alpha * 255.0) as u8,
            });
        }
    }
    Pixmap::from_parts_with_opacity(pixels, size, size, true)
}

fn bounce(position: &mut f64, velocity: &mut f64, max: f64) {
    if *position < 0.0 {
        *position = 0.0;
        *velocity = velocity.abs();
    } else if *position > max {
        *position = max;
        *velocity = -velocity.abs();
    }
}

#[derive(Debug)]
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut state = self.state;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.state = state;
        state
    }

    fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn color(&mut self) -> [u8; 3] {
        [
            self.next_u64() as u8,
            self.next_u64() as u8,
            self.next_u64() as u8,
        ]
    }
}
