use std::cell::RefCell;
use std::rc::Rc;

use glifo::Glyph;
use vello::peniko::color::palette;
use vello::peniko::{Brush, Fill, FontData};
use vello::wgpu::{self, CurrentSurfaceTexture};
use vello_common::filter_effects::Filter;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::pixmap::Pixmap;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlCanvasElement;

use crate::backend::{Backend, BackendKind, layout_text_glyphs};
use crate::capability::CapabilityProfile;
use crate::scenes::{ParamId, SceneId};

pub(crate) const CAPABILITIES: CapabilityProfile = CapabilityProfile::all()
    .deny_scenes(&[SceneId::FilterLayers])
    .deny_params(SceneId::Rect, &[ParamId::ImageFilter, ParamId::UseDrawImage]);

type SceneBrush = Brush<vello::peniko::ImageBrush, vello::peniko::Gradient>;

pub struct BackendImpl {
    scene: vello::Scene,
    inner: Rc<RefCell<Option<Inner>>>,
    width: u32,
    height: u32,
    transform: Affine,
    paint_transform: Affine,
    stroke: Stroke,
    fill: Fill,
    brush: SceneBrush,
}

struct Inner {
    context: vello::util::RenderContext,
    surface: vello::util::RenderSurface<'static>,
    renderer: vello::Renderer,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(vello)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        let inner = Rc::new(RefCell::new(None));
        spawn_init(inner.clone(), canvas.clone(), w, h);

        Self {
            scene: vello::Scene::new(),
            inner,
            width: w,
            height: h,
            transform: Affine::IDENTITY,
            paint_transform: Affine::IDENTITY,
            stroke: Stroke::new(1.0),
            fill: Fill::NonZero,
            brush: palette::css::BLACK.into(),
        }
    }

    fn brush_transform(&self) -> Option<Affine> {
        (self.paint_transform != Affine::IDENTITY).then_some(self.paint_transform)
    }

    fn draw_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]) {
        self.scene
            .draw_glyphs(font)
            .transform(self.transform)
            .font_size(font_size)
            .hint(hint)
            .brush(&self.brush)
            .draw(
                self.fill,
                glyphs.iter().copied().map(|glyph| vello::Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                }),
            );
    }
}

fn spawn_init(
    slot: Rc<RefCell<Option<Inner>>>,
    canvas: HtmlCanvasElement,
    width: u32,
    height: u32,
) {
    spawn_local(async move {
        let mut context = vello::util::RenderContext::new();
        let mut surface = match context
            .create_surface(
                wgpu::SurfaceTarget::Canvas(canvas),
                width,
                height,
                wgpu::PresentMode::AutoVsync,
            )
            .await
        {
            Ok(surface) => surface,
            Err(error) => {
                log::warn!("Failed to initialize Vello surface: {error:?}");
                return;
            }
        };
        surface.config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        context.configure_surface(&surface);
        let device = &context.devices[surface.dev_id].device;
        let renderer = match vello::Renderer::new(
            device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                ..vello::RendererOptions::default()
            },
        ) {
            Ok(renderer) => renderer,
            Err(error) => {
                log::warn!("Failed to initialize Vello renderer: {error:?}");
                return;
            }
        };

        *slot.borrow_mut() = Some(Inner {
            context,
            surface,
            renderer,
        });
    });
}

impl Backend for BackendImpl {
    fn kind(&self) -> BackendKind {
        BackendKind::Vello
    }

    fn reset(&mut self) {
        self.scene.reset();
    }

    fn render_offscreen(&mut self) {
        let mut inner_slot = self.inner.borrow_mut();
        let Some(inner) = inner_slot.as_mut() else {
            return;
        };
        let device_handle = &inner.context.devices[inner.surface.dev_id];
        let params = vello::RenderParams {
            base_color: palette::css::TRANSPARENT,
            width: self.width,
            height: self.height,
            antialiasing_method: vello::AaConfig::Area,
        };
        if let Err(error) = inner.renderer.render_to_texture(
            &device_handle.device,
            &device_handle.queue,
            &self.scene,
            &inner.surface.target_view,
            &params,
        ) {
            log::warn!("Vello render failed: {error:?}");
        }
    }

    fn blit(&mut self) {
        let mut inner_slot = self.inner.borrow_mut();
        let Some(inner) = inner_slot.as_mut() else {
            return;
        };
        let device_handle = &inner.context.devices[inner.surface.dev_id];
        let surface_texture = match inner.surface.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(surface_texture) => surface_texture,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Suboptimal(_) => {
                inner.context.configure_surface(&inner.surface);
                return;
            }
            CurrentSurfaceTexture::Occluded | CurrentSurfaceTexture::Timeout => return,
            CurrentSurfaceTexture::Lost => {
                inner.context.configure_surface(&inner.surface);
                return;
            }
            CurrentSurfaceTexture::Validation => return,
        };

        let mut encoder =
            device_handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Vello surface blit"),
                });
        inner.surface.blitter.copy(
            &device_handle.device,
            &mut encoder,
            &inner.surface.target_view,
            &surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        device_handle.queue.submit([encoder.finish()]);
        surface_texture.present();
        let _ = device_handle.device.poll(wgpu::PollType::Poll);
    }

    fn is_cpu(&self) -> bool {
        false
    }

    fn supports_encode_timing(&self) -> bool {
        true
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        if let Some(inner) = self.inner.borrow_mut().as_mut() {
            inner.context.resize_surface(&mut inner.surface, w, h);
        }
    }

    fn set_paint(&mut self, paint: PaintType) {
        self.brush = match paint {
            Brush::Solid(color) => Brush::Solid(color),
            Brush::Gradient(gradient) => Brush::Gradient(gradient),
            Brush::Image(_) => palette::css::BLACK.into(),
        };
    }

    fn set_transform(&mut self, transform: Affine) {
        self.transform = transform;
    }

    fn reset_transform(&mut self) {
        self.transform = Affine::IDENTITY;
    }

    fn set_stroke(&mut self, stroke: Stroke) {
        self.stroke = stroke;
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        self.paint_transform = transform;
    }

    fn reset_paint_transform(&mut self) {
        self.paint_transform = Affine::IDENTITY;
    }

    fn set_fill_rule(&mut self, fill: Fill) {
        self.fill = fill;
    }

    fn fill_rect(&mut self, rect: &Rect) {
        self.scene.fill(
            self.fill,
            self.transform,
            &self.brush,
            self.brush_transform(),
            rect,
        );
    }

    fn fill_path(&mut self, path: &BezPath) {
        self.scene.fill(
            self.fill,
            self.transform,
            &self.brush,
            self.brush_transform(),
            path,
        );
    }

    fn stroke_path(&mut self, path: &BezPath) {
        self.scene.stroke(
            &self.stroke,
            self.transform,
            &self.brush,
            self.brush_transform(),
            path,
        );
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.scene
            .push_clip_layer(self.fill, self.transform, path);
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.scene
            .push_clip_layer(self.fill, self.transform, path);
    }

    fn set_filter_effect(&mut self, _filter: Filter) {}

    fn pop_clip_path(&mut self) {
        self.scene.pop_layer();
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
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

    fn draw_image(&mut self, _image: ImageSource, _rect: &Rect, _bilinear: bool) {}

    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        ImageSource::Pixmap(std::sync::Arc::new(pixmap))
    }

    fn destroy_image(&mut self, _image: &ImageSource) {}
}
