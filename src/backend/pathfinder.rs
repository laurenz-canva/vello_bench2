use pathfinder_canvas::{Canvas, CanvasFontContext, CanvasRenderingContext2D, ColorF, ColorU, RectF};
use pathfinder_geometry::transform2d::Transform2F;
use pathfinder_geometry::vector::{Vector2F, vec2i};
use pathfinder_renderer::concurrent::executor::SequentialExecutor;
use pathfinder_renderer::gpu::options::{DestFramebuffer, RendererMode, RendererOptions};
use pathfinder_renderer::gpu::renderer::Renderer as PathfinderRenderer;
use pathfinder_renderer::options::BuildOptions;
use pathfinder_resources::embedded::EmbeddedResourceLoader;
use pathfinder_webgl::WebGlDevice;
use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use crate::scenes::{ParamId, SceneId};

pub struct Pixmap;

impl Pixmap {
    pub fn from_parts_with_opacity<T>(
        _pixels: Vec<T>,
        _width: u16,
        _height: u16,
        _may_have_opacities: bool,
    ) -> Self {
        Self
    }
}

pub fn supports_scene(scene_id: SceneId) -> bool {
    matches!(scene_id, SceneId::Rect)
}

pub fn supports_param(scene_id: SceneId, param: ParamId) -> bool {
    matches!(
        (scene_id, param),
        (SceneId::Rect, ParamId::NumRects)
            | (SceneId::Rect, ParamId::RectSize)
            | (SceneId::Rect, ParamId::Rotated)
    )
}

pub struct BackendImpl {
    ctx: DrawContext,
    renderer: PathfinderRenderer<WebGlDevice>,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(pathfinder)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        let context: WebGl2RenderingContext = canvas
            .get_context("webgl2")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let device = WebGlDevice::new(context);
        let framebuffer_size = vec2i(canvas.width() as i32, canvas.height() as i32);
        let mode = RendererMode::default_for_device(&device);
        let options = RendererOptions {
            dest: DestFramebuffer::full_window(framebuffer_size),
            background_color: Some(ColorF::new(
                17.0 / 255.0,
                17.0 / 255.0,
                27.0 / 255.0,
                1.0,
            )),
            ..RendererOptions::default()
        };
        let loader = EmbeddedResourceLoader::new();
        Self {
            ctx: DrawContext::new(w as u16, h as u16),
            renderer: PathfinderRenderer::new(device, &loader, mode, options),
        }
    }

    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    pub fn reset_with_size(&mut self, w: u32, h: u32) {
        self.ctx = DrawContext::new(w as u16, h as u16);
    }

    pub fn render_offscreen(&mut self) {
        if let Some(canvas) = self.ctx.canvas.take() {
            let mut scene = canvas.into_canvas().into_scene();
            scene.build_and_render(
                &mut self.renderer,
                BuildOptions::default(),
                SequentialExecutor,
            );
        }
    }

    pub fn blit(&mut self) {}

    pub fn is_cpu(&self) -> bool {
        false
    }

    pub fn sync(&self) {}

    pub fn resize(&mut self, w: u32, h: u32) {
        self.ctx = DrawContext::new(w as u16, h as u16);
        self.renderer.options_mut().dest = DestFramebuffer::full_window(vec2i(w as i32, h as i32));
        self.renderer.dest_framebuffer_size_changed();
    }

    pub fn upload_image(&mut self, _pixmap: Pixmap) -> ImageSource {
        panic!("pathfinder image upload not implemented")
    }

    pub fn set_paint(&mut self, paint: PaintType) {
        self.ctx.set_paint(paint);
    }

    pub fn set_transform(&mut self, transform: Affine) {
        self.ctx.set_transform(transform);
    }

    pub fn reset_transform(&mut self) {
        self.ctx.reset_transform();
    }

    pub fn set_stroke(&mut self, _stroke: Stroke) {}

    pub fn set_paint_transform(&mut self, _transform: Affine) {}

    pub fn reset_paint_transform(&mut self) {}

    pub fn set_fill_rule(&mut self, _fill: Fill) {}

    pub fn fill_rect(&mut self, rect: &Rect) {
        self.ctx.fill_rect(rect);
    }

    pub fn fill_path(&mut self, _path: &BezPath) {}

    pub fn stroke_path(&mut self, _path: &BezPath) {}

    pub fn push_clip_path(&mut self, _path: &BezPath) {}

    pub fn push_clip_layer(&mut self, _path: &BezPath) {}

    pub fn push_filter_layer(&mut self, _filter: Filter) {}

    pub fn pop_clip_path(&mut self) {}

    pub fn pop_layer(&mut self) {}

    pub fn fill_glyphs(&mut self, _font: &FontData, _font_size: f32, _hint: bool, _glyphs: &[Glyph]) {}

    pub fn draw_image(&mut self, _image: ImageSource, _rect: &Rect, _bilinear: bool) {}
}

struct DrawContext {
    width: u16,
    height: u16,
    canvas: Option<CanvasRenderingContext2D>,
    fill_color: ColorU,
}

impl DrawContext {
    fn new(width: u16, height: u16) -> Self {
        let mut ctx = Self {
            width,
            height,
            canvas: None,
            fill_color: ColorU::black(),
        };
        ctx.reset();
        ctx
    }

    fn reset(&mut self) {
        let font_context = CanvasFontContext::from_system_source();
        self.canvas = Some(
            Canvas::new(Vector2F::new(self.width as f32, self.height as f32))
                .get_context_2d(font_context),
        );
    }

    fn set_paint(&mut self, paint: PaintType) {
        if let PaintType::Solid(color) = paint {
            let [r, g, b, a] = color.to_rgba8().to_u8_array();
            self.fill_color = ColorU::new(r, g, b, a);
        }
    }

    fn set_transform(&mut self, transform: Affine) {
        if let Some(canvas) = self.canvas.as_mut() {
            let c = transform.as_coeffs();
            canvas.set_transform(&Transform2F::row_major(
                c[0] as f32,
                c[2] as f32,
                c[4] as f32,
                c[1] as f32,
                c[3] as f32,
                c[5] as f32,
            ));
        }
    }

    fn reset_transform(&mut self) {
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.reset_transform();
        }
    }

    fn fill_rect(&mut self, rect: &Rect) {
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.set_fill_style(self.fill_color);
            canvas.fill_rect(RectF::new(
                Vector2F::new(rect.x0 as f32, rect.y0 as f32),
                Vector2F::new(rect.width() as f32, rect.height() as f32),
            ));
        }
    }
}
