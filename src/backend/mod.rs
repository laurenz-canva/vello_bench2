//! Backend abstraction over vello_hybrid, vello_cpu, Pathfinder, and Canvas 2D.

mod canvas2d;
mod cpu;
mod hybrid;
#[cfg(feature = "pathfinder")]
mod pathfinder;
#[cfg(feature = "vello")]
mod vello;

use glifo::Glyph;
use skrifa::MetadataProvider;
use skrifa::raw::FileRef;
use vello_common::filter_effects::Filter;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
pub use vello_common::paint::ImageSource;
use vello_common::paint::{ImageId, PaintType};
use vello_common::peniko::{Fill, FontData};
use web_sys::HtmlCanvasElement;

use crate::capability::CapabilityProfile;
use crate::scenes::{ParamId, SceneId};

pub use vello_common::pixmap::Pixmap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Hybrid,
    #[cfg(feature = "vello")]
    Vello,
    Cpu,
    #[cfg(feature = "pathfinder")]
    Pathfinder,
    Canvas2d,
    Canvas2dCpu,
}

impl BackendKind {
    #[cfg(all(feature = "pathfinder", feature = "vello"))]
    pub const ALL: [Self; 6] = [
        Self::Hybrid,
        Self::Vello,
        Self::Cpu,
        Self::Pathfinder,
        Self::Canvas2d,
        Self::Canvas2dCpu,
    ];
    #[cfg(all(feature = "pathfinder", not(feature = "vello")))]
    pub const ALL: [Self; 5] = [
        Self::Hybrid,
        Self::Cpu,
        Self::Pathfinder,
        Self::Canvas2d,
        Self::Canvas2dCpu,
    ];
    #[cfg(all(not(feature = "pathfinder"), feature = "vello"))]
    pub const ALL: [Self; 5] = [
        Self::Hybrid,
        Self::Vello,
        Self::Cpu,
        Self::Canvas2d,
        Self::Canvas2dCpu,
    ];
    #[cfg(all(not(feature = "pathfinder"), not(feature = "vello")))]
    pub const ALL: [Self; 4] = [Self::Hybrid, Self::Cpu, Self::Canvas2d, Self::Canvas2dCpu];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            #[cfg(feature = "vello")]
            Self::Vello => "vello",
            Self::Cpu => "cpu",
            #[cfg(feature = "pathfinder")]
            Self::Pathfinder => "pathfinder",
            Self::Canvas2d => "canvas2d",
            Self::Canvas2dCpu => "canvas2d_cpu",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hybrid => "Vello Hybrid",
            #[cfg(feature = "vello")]
            Self::Vello => "Vello",
            Self::Cpu => "Vello CPU",
            #[cfg(feature = "pathfinder")]
            Self::Pathfinder => "Pathfinder",
            Self::Canvas2d => "Canvas 2D",
            Self::Canvas2dCpu => "Canvas 2D (CPU)",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "hybrid" => Some(Self::Hybrid),
            #[cfg(feature = "vello")]
            "vello" | "webgpu" => Some(Self::Vello),
            "cpu" => Some(Self::Cpu),
            #[cfg(feature = "pathfinder")]
            "pathfinder" => Some(Self::Pathfinder),
            "canvas2d" => Some(Self::Canvas2d),
            "canvas2d_cpu" | "canvas2d_software" => Some(Self::Canvas2dCpu),
            _ => None,
        }
    }

    fn capabilities(self) -> &'static CapabilityProfile {
        match self {
            Self::Hybrid => &hybrid::CAPABILITIES,
            #[cfg(feature = "vello")]
            Self::Vello => &vello::CAPABILITIES,
            Self::Cpu => &cpu::CAPABILITIES,
            #[cfg(feature = "pathfinder")]
            Self::Pathfinder => &pathfinder::CAPABILITIES,
            Self::Canvas2d | Self::Canvas2dCpu => &canvas2d::CAPABILITIES,
        }
    }

    pub fn is_available(self) -> bool {
        match self {
            #[cfg(feature = "vello")]
            Self::Vello => webgpu_supported(),
            _ => true,
        }
    }

    pub fn available() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|kind| kind.is_available())
    }
}

pub fn current_backend_kind() -> BackendKind {
    crate::storage::load_backend_name()
        .as_deref()
        .and_then(BackendKind::from_str)
        .filter(|kind| kind.is_available())
        .unwrap_or(BackendKind::Hybrid)
}

#[cfg(feature = "vello")]
pub fn webgpu_supported() -> bool {
    web_sys::window()
        .and_then(|window| {
            js_sys::Reflect::get(window.as_ref(), &wasm_bindgen::JsValue::from_str("navigator"))
                .ok()
        })
        .and_then(|navigator| {
            js_sys::Reflect::get(&navigator, &wasm_bindgen::JsValue::from_str("gpu")).ok()
        })
        .is_some_and(|gpu| !gpu.is_undefined() && !gpu.is_null())
}

pub trait Backend {
    fn kind(&self) -> BackendKind;
    fn reset(&mut self);
    fn render_offscreen(&mut self);
    fn blit(&mut self);
    fn is_cpu(&self) -> bool;
    fn supports_encode_timing(&self) -> bool;
    fn resize(&mut self, w: u32, h: u32);
    fn set_paint(&mut self, paint: PaintType);
    fn set_transform(&mut self, transform: Affine);
    fn reset_transform(&mut self);
    fn set_stroke(&mut self, stroke: Stroke);
    fn set_paint_transform(&mut self, transform: Affine);
    fn reset_paint_transform(&mut self);
    fn set_fill_rule(&mut self, fill: Fill);
    fn fill_rect(&mut self, rect: &Rect);
    fn fill_path(&mut self, path: &BezPath);
    fn stroke_path(&mut self, path: &BezPath);
    fn push_clip_path(&mut self, path: &BezPath);
    fn push_clip_layer(&mut self, path: &BezPath);
    fn set_filter_effect(&mut self, filter: Filter);
    fn pop_clip_path(&mut self);
    fn pop_layer(&mut self);
    fn draw_text(
        &mut self,
        font: &FontData,
        font_size: f32,
        hint: bool,
        text: &str,
        x: f32,
        y: f32,
    );
    fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool);
    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource;
    fn destroy_image(&mut self, image: &ImageSource);
    fn probe(&mut self) -> Result<(), String> {
        Err("Backend probing is only supported for Vello Hybrid".to_string())
    }
}

pub fn uploaded_image_id(image: &ImageSource) -> Option<ImageId> {
    match image {
        ImageSource::OpaqueId { id, .. } => Some(*id),
        ImageSource::Pixmap(_) => None,
    }
}

pub fn layout_text_glyphs(
    font: &FontData,
    font_size: f32,
    text: &str,
    x: f32,
    y: f32,
) -> Vec<Glyph> {
    let font_ref = match FileRef::new(font.data.as_ref()).unwrap() {
        FileRef::Font(f) => f,
        FileRef::Collection(c) => c.get(font.index).unwrap(),
    };
    let size = skrifa::instance::Size::new(font_size);
    let charmap = font_ref.charmap();
    let glyph_metrics = font_ref.glyph_metrics(size, skrifa::instance::LocationRef::default());
    let mut pen_x = x;
    let mut glyphs = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let gid = charmap.map(ch).unwrap_or_default();
        glyphs.push(Glyph {
            id: gid.to_u32(),
            x: pen_x,
            y,
        });
        pen_x += glyph_metrics.advance_width(gid).unwrap_or_default();
    }
    glyphs
}

#[derive(Debug, Clone, Copy)]
pub struct BackendCapabilities {
    kind: BackendKind,
}

impl BackendCapabilities {
    pub fn supports_scene(self, scene_id: SceneId) -> bool {
        self.kind.capabilities().supports_scene(scene_id)
    }

    pub fn supports_param(self, scene_id: SceneId, param: ParamId) -> bool {
        self.kind.capabilities().supports_param(scene_id, param)
    }

    pub fn supports_param_value(self, scene_id: SceneId, param: ParamId, value: f64) -> bool {
        self.kind
            .capabilities()
            .supports_param_value(scene_id, param, value)
    }
}

pub fn current_backend_capabilities(kind: BackendKind) -> BackendCapabilities {
    BackendCapabilities { kind }
}

pub fn new_backend(
    canvas: &HtmlCanvasElement,
    w: u32,
    h: u32,
    kind: BackendKind,
) -> Box<dyn Backend> {
    match kind {
        BackendKind::Hybrid => Box::new(hybrid::BackendImpl::new(canvas, w, h)),
        #[cfg(feature = "vello")]
        BackendKind::Vello => Box::new(vello::BackendImpl::new(canvas, w, h)),
        BackendKind::Cpu => Box::new(cpu::BackendImpl::new(canvas, w, h)),
        #[cfg(feature = "pathfinder")]
        BackendKind::Pathfinder => Box::new(pathfinder::BackendImpl::new(canvas, w, h)),
        BackendKind::Canvas2d => Box::new(canvas2d::BackendImpl::new(canvas, w, h, kind)),
        BackendKind::Canvas2dCpu => Box::new(canvas2d::BackendImpl::new(canvas, w, h, kind)),
    }
}
