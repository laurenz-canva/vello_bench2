use glifo::Glyph;
use vello_common::TextureId;
use vello_common::filter_effects::Filter;
use vello_common::geometry::RectU16;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::multi_atlas::AtlasConfig;
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use vello_common::pixmap::Pixmap;
use vello_hybrid::{LayersConfig, MemorySettings, WebGlTextureBindings};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use crate::backend::{Backend, BackendKind, layout_text_glyphs, uploaded_image_id};
use crate::capability::CapabilityProfile;

pub(crate) const CAPABILITIES: CapabilityProfile = CapabilityProfile::all();

pub struct BackendImpl {
    ctx: vello_hybrid::Scene,
    resources: vello_hybrid::Resources,
    renderer: Option<vello_hybrid::WebGlRenderer>,
    renderer_init: Option<vello_hybrid::WebGlRendererInit>,
    external_texture_bindings: WebGlTextureBindings,
    next_external_texture_id: u64,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(hybrid)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        let image_atlas_config = AtlasConfig::default();
        let memory_settings = MemorySettings {
            layers_config: LayersConfig::default(),
            image_atlas_config,
        };
        let settings = vello_hybrid::RenderSettings {
            memory_settings,
            ..Default::default()
        };

        let (renderer_init, resources) =
            vello_hybrid::WebGlRenderer::begin_with(canvas, settings, true);

        Self {
            ctx: vello_hybrid::Scene::new(w as u16, h as u16),
            resources,
            renderer: None,
            renderer_init: Some(renderer_init),
            external_texture_bindings: WebGlTextureBindings::new(),
            next_external_texture_id: 0,
        }
    }

    fn draw_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]) {
        self.ctx
            .glyph_run(&mut self.resources, font)
            .font_size(font_size)
            .hint(hint)
            .fill_glyphs(glyphs.iter().copied());
    }
}

impl Backend for BackendImpl {
    fn kind(&self) -> BackendKind {
        BackendKind::Hybrid
    }

    fn poll_ready(&mut self) -> bool {
        if self.renderer.is_some() {
            return true;
        }
        let Some(init) = self.renderer_init.take() else {
            return false;
        };
        match init.try_finish() {
            vello_hybrid::WebGlRendererInitStatus::Pending(init) => {
                self.renderer_init = Some(init);
                false
            }
            vello_hybrid::WebGlRendererInitStatus::Complete(renderer) => {
                self.renderer = Some(renderer);
                true
            }
        }
    }

    fn reset(&mut self) {
        self.ctx.reset();
    }

    fn render_offscreen(&mut self) {
        let rs = vello_hybrid::RenderSize {
            width: self.ctx.width() as u32,
            height: self.ctx.height() as u32,
        };
        self.renderer
            .as_mut()
            .expect("WebGL renderer used before initialization completed")
            .render(
                &self.ctx,
                &mut self.resources,
                &rs,
                &self.external_texture_bindings,
            )
            .unwrap();
    }

    fn blit(&mut self) {}

    fn is_cpu(&self) -> bool {
        false
    }

    fn supports_encode_timing(&self) -> bool {
        true
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.ctx = vello_hybrid::Scene::new(w as u16, h as u16);
    }

    fn set_paint(&mut self, paint: PaintType) {
        self.ctx.set_paint(paint);
    }

    fn set_transform(&mut self, transform: Affine) {
        self.ctx.set_transform(transform);
    }

    fn reset_transform(&mut self) {
        self.ctx.reset_transform();
    }

    fn set_stroke(&mut self, stroke: Stroke) {
        self.ctx.set_stroke(stroke);
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        self.ctx.set_paint_transform(transform);
    }

    fn reset_paint_transform(&mut self) {
        self.ctx.reset_paint_transform();
    }

    fn set_fill_rule(&mut self, fill: Fill) {
        self.ctx.set_fill_rule(fill);
    }

    fn fill_rect(&mut self, rect: &Rect) {
        self.ctx.fill_rect(rect);
    }

    fn fill_path(&mut self, path: &BezPath) {
        self.ctx.fill_path(path);
    }

    fn stroke_path(&mut self, path: &BezPath) {
        self.ctx.stroke_path(path);
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.ctx.push_clip_path(path);
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.ctx.push_clip_layer(path);
    }

    fn set_filter_effect(&mut self, filter: Filter) {
        self.ctx.push_filter_layer(filter);
    }

    fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }

    fn draw_text(
        &mut self,
        font: &FontData,
        font_size: f32,
        hint: bool,
        text: &str,
        x: f32,
        y: f32,
    ) {
        let glyphs = layout_text_glyphs(font, font_size, text, x, y);
        self.draw_glyphs(font, font_size, hint, &glyphs);
    }

    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        let may_have_transparency = pixmap.may_have_transparency();
        let renderer = self
            .renderer
            .as_mut()
            .expect("WebGL renderer used before initialization completed");
        let id = renderer.upload_image(&mut self.resources, &pixmap);
        ImageSource::opaque_id_with_transparency_hint(id, may_have_transparency)
    }

    fn upload_external_image(&mut self, pixmap: Pixmap) -> ImageSource {
        let width = pixmap.width();
        let height = pixmap.height();
        let may_have_transparency = pixmap.may_have_transparency();
        let renderer = self
            .renderer
            .as_ref()
            .expect("WebGL renderer used before initialization completed");
        let gl = renderer.gl_context();
        let texture = gl
            .create_texture()
            .expect("failed to create external texture");
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&texture));
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_MIN_FILTER,
            WebGl2RenderingContext::NEAREST as i32,
        );
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_MAG_FILTER,
            WebGl2RenderingContext::NEAREST as i32,
        );
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_WRAP_S,
            WebGl2RenderingContext::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_WRAP_T,
            WebGl2RenderingContext::CLAMP_TO_EDGE as i32,
        );
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            WebGl2RenderingContext::TEXTURE_2D,
            0,
            WebGl2RenderingContext::RGBA8 as i32,
            i32::from(width),
            i32::from(height),
            0,
            WebGl2RenderingContext::RGBA,
            WebGl2RenderingContext::UNSIGNED_BYTE,
            Some(pixmap.data_as_u8_slice()),
        )
        .expect("failed to upload external texture");
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);

        let texture_id = TextureId(self.next_external_texture_id);
        self.next_external_texture_id = self.next_external_texture_id.wrapping_add(1);
        self.external_texture_bindings.insert(texture_id, texture);
        ImageSource::external_texture(
            texture_id,
            RectU16::new(0, 0, width, height),
            may_have_transparency,
        )
    }

    fn destroy_image(&mut self, image: &ImageSource) {
        if let ImageSource::ExternalTexture { id, .. } = image {
            if let Some(texture) = self.external_texture_bindings.remove(*id)
                && let Some(renderer) = self.renderer.as_ref()
            {
                renderer.gl_context().delete_texture(Some(&texture));
            }
            return;
        }
        if let Some(id) = uploaded_image_id(image)
            && let Some(renderer) = self.renderer.as_mut()
        {
            renderer.destroy_image(&mut self.resources, id);
        }
    }

    fn probe(&mut self) -> Result<vello_hybrid::WebGlPendingProbe, String> {
        self.renderer
            .as_mut()
            .ok_or_else(|| "WebGL initialization is still in progress".to_string())?
            .probe()
            .map_err(|error| error.to_string())
    }
}
