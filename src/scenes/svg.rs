//! Vector graphics (SVG) benchmark scene using usvg for proper parsing.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamId, ParamKind, SceneId};
use crate::backend::{Backend, Pixmap};
use crate::resource_store::ResourceStore;
use std::cell::RefCell;
use std::io::{Cursor, Read};
use std::rc::Rc;
use usvg::tiny_skia_path::PathSegment;
use usvg::{Group, ImageKind, ImageRendering, Node};
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::Image as PaintImage;
use vello_common::peniko::{Color, Extend, ImageQuality, ImageSampler, color::PremulRgba8};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};

const SVG_IMAGE_EPOCH: u64 = 0;

#[derive(Clone, Copy)]
struct SvgAssetDesc {
    name: &'static str,
    path: &'static str,
}

const SVG_ASSETS: &[SvgAssetDesc] = &[
    SvgAssetDesc {
        name: "Ghostscript Tiger",
        path: "Ghostscript_Tiger.svg.br",
    },
    SvgAssetDesc {
        name: "Coat of Arms",
        path: "coat_of_arms.svg.br",
    },
    SvgAssetDesc {
        name: "Heraldry",
        path: "heraldry.svg.br",
    },
];

thread_local! {
    static ASSET_CACHE: RefCell<Vec<AssetState>> =
        RefCell::new((0..SVG_ASSETS.len()).map(|_| AssetState::NotLoaded).collect());
}

enum AssetState {
    NotLoaded,
    Loading,
    Ready(Rc<SvgAsset>),
    Failed(String),
}

/// A single draw command in document order.
enum DrawCmd {
    Fill {
        path: BezPath,
        transform: Affine,
        color: Color,
    },
    Stroke {
        path: BezPath,
        transform: Affine,
        color: Color,
        width: f64,
    },
    PushClip {
        path: BezPath,
        transform: Affine,
    },
    Image {
        image_idx: usize,
        transform: Affine,
        width: f64,
        height: f64,
        bilinear: bool,
    },
    PopClip,
}

/// A decoded raster image embedded in an SVG asset.
struct SvgImage {
    cache_key: u64,
    pixmap: Pixmap,
}

/// A pre-parsed SVG asset ready for rendering.
struct SvgAsset {
    commands: Vec<DrawCmd>,
    images: Vec<SvgImage>,
    width: f64,
    height: f64,
}

/// Benchmark scene that renders one of several SVG assets.
pub struct SvgScene {
    selected: usize,
}

impl std::fmt::Debug for SvgScene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgScene")
            .field("selected", &self.selected)
            .finish()
    }
}

impl SvgScene {
    /// Create a new SVG scene. The selected asset is fetched and parsed on demand.
    pub fn new() -> Self {
        Self { selected: 0 }
    }

    fn selected_asset(&mut self) -> Option<Rc<SvgAsset>> {
        self.ensure_selected_loading();
        ASSET_CACHE.with(|cache| match &cache.borrow()[self.selected] {
            AssetState::Ready(asset) => Some(asset.clone()),
            AssetState::Failed(err) => {
                log::error!(
                    "Failed to load SVG asset {}: {err}",
                    SVG_ASSETS[self.selected].name
                );
                None
            }
            AssetState::NotLoaded | AssetState::Loading => None,
        })
    }

    fn ensure_selected_loading(&mut self) {
        let idx = self.selected;
        let should_load = ASSET_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            match cache.get_mut(idx) {
                Some(state @ AssetState::NotLoaded) => {
                    *state = AssetState::Loading;
                    true
                }
                _ => false,
            }
        });
        if !should_load {
            return;
        }

        spawn_local(async move {
            let desc = SVG_ASSETS[idx];
            let result = load_asset(idx, desc).await.map(Rc::new);
            ASSET_CACHE.with(|cache| {
                cache.borrow_mut()[idx] = match result {
                    Ok(asset) => AssetState::Ready(asset),
                    Err(err) => AssetState::Failed(err),
                };
            });
        });
    }
}

fn parse_asset(asset_idx: usize, desc: SvgAssetDesc, data: &[u8]) -> Result<SvgAsset, String> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())
        .map_err(|e| format!("failed to parse {}: {e}", desc.name))?;
    let mut commands = Vec::new();
    let mut images = Vec::new();
    convert_group(
        &mut commands,
        &mut images,
        tree.root(),
        Affine::IDENTITY,
        (asset_idx as u64) << 32,
    )?;
    Ok(SvgAsset {
        commands,
        images,
        width: tree.size().width() as f64,
        height: tree.size().height() as f64,
    })
}

async fn load_asset(asset_idx: usize, desc: SvgAssetDesc) -> Result<SvgAsset, String> {
    let compressed = fetch_asset(desc.path).await?;
    let mut decompressed = Vec::new();
    let mut decoder = brotli_decompressor::Decompressor::new(Cursor::new(compressed), 4096);
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| format!("failed to decompress {}: {e}", desc.name))?;
    parse_asset(asset_idx, desc, &decompressed)
}

async fn fetch_asset(path: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("window is unavailable")?;
    let url = format!("{}{}", asset_base_url(), path);
    let response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .map_err(|e| format!("failed to fetch {url}: {e:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| format!("fetch did not return a Response for {url}"))?;
    if !response.ok() {
        return Err(format!(
            "fetch failed for {url}: HTTP {}",
            response.status()
        ));
    }
    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| format!("failed to read {url}: {e:?}"))?,
    )
    .await
    .map_err(|e| format!("failed to read {url}: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

fn asset_base_url() -> String {
    let base = js_sys::Reflect::get(&js_sys::global(), &"__vello_asset_base".into())
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| "./assets/".to_string());
    if base.ends_with('/') {
        base
    } else {
        format!("{base}/")
    }
}

// ── usvg → draw command conversion ──────────────────────────────────────────

fn convert_group(
    commands: &mut Vec<DrawCmd>,
    images: &mut Vec<SvgImage>,
    g: &Group,
    parent_transform: Affine,
    image_key_base: u64,
) -> Result<(), String> {
    let transform = parent_transform * convert_transform(&g.transform());

    // Handle clip path on this group.
    let has_clip = if let Some(clip) = g.clip_path() {
        let clip_transform = transform * convert_transform(&clip.transform());
        let clip_path = flatten_group_to_path(clip.root());
        if !clip_path.elements().is_empty() {
            commands.push(DrawCmd::PushClip {
                path: clip_path,
                transform: clip_transform,
            });
        }
        true
    } else {
        false
    };

    for child in g.children() {
        match child {
            Node::Group(group) => {
                convert_group(commands, images, group, transform, image_key_base)?;
            }
            Node::Path(p) => {
                let bez = convert_path(p);

                if let Some(fill) = p.fill() {
                    let color = usvg_paint_to_color(&fill.paint(), fill.opacity());
                    commands.push(DrawCmd::Fill {
                        path: bez.clone(),
                        transform,
                        color,
                    });
                }

                if let Some(stroke) = p.stroke() {
                    let color = usvg_paint_to_color(&stroke.paint(), stroke.opacity());
                    commands.push(DrawCmd::Stroke {
                        path: bez,
                        transform,
                        color,
                        width: stroke.width().get() as f64,
                    });
                }
            }
            Node::Image(image) => {
                convert_image(commands, images, image, transform, image_key_base)?;
            }
            Node::Text(_) => {}
        }
    }

    if has_clip {
        commands.push(DrawCmd::PopClip);
    }

    Ok(())
}

fn convert_image(
    commands: &mut Vec<DrawCmd>,
    images: &mut Vec<SvgImage>,
    image: &usvg::Image,
    transform: Affine,
    image_key_base: u64,
) -> Result<(), String> {
    if !image.is_visible() {
        return Ok(());
    }

    match image.kind() {
        ImageKind::SVG(tree) => {
            convert_group(commands, images, tree.root(), transform, image_key_base)?;
        }
        kind => {
            let Some(pixmap) = decode_raster_image(kind)? else {
                return Ok(());
            };
            let image_idx = images.len();
            let size = image.size();
            commands.push(DrawCmd::Image {
                image_idx,
                transform,
                width: size.width() as f64,
                height: size.height() as f64,
                bilinear: image_bilinear(image.rendering_mode()),
            });
            images.push(SvgImage {
                cache_key: image_key_base | image_idx as u64,
                pixmap,
            });
        }
    }

    Ok(())
}

fn decode_raster_image(kind: &ImageKind) -> Result<Option<Pixmap>, String> {
    let (data, format) = match kind {
        ImageKind::JPEG(data) => (data.as_slice(), image::ImageFormat::Jpeg),
        ImageKind::PNG(data) => (data.as_slice(), image::ImageFormat::Png),
        ImageKind::GIF(data) => (data.as_slice(), image::ImageFormat::Gif),
        ImageKind::WEBP(data) => (data.as_slice(), image::ImageFormat::WebP),
        ImageKind::SVG(_) => return Ok(None),
    };
    let rgba = image::load_from_memory_with_format(data, format)
        .map_err(|e| format!("failed to decode embedded SVG image: {e}"))?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    let width: u16 = width
        .try_into()
        .map_err(|_| format!("embedded SVG image width {width} exceeds u16::MAX"))?;
    let height: u16 = height
        .try_into()
        .map_err(|_| format!("embedded SVG image height {height} exceeds u16::MAX"))?;

    let mut may_have_opacities = false;
    let pixels = rgba
        .into_raw()
        .chunks_exact(4)
        .map(|rgba| {
            let [r, g, b, a]: [u8; 4] = rgba.try_into().unwrap();
            if a != 255 {
                may_have_opacities = true;
            }
            let alpha = u16::from(a);
            let premultiply = |channel: u8| ((u16::from(channel) * alpha) / 255) as u8;
            PremulRgba8 {
                r: premultiply(r),
                g: premultiply(g),
                b: premultiply(b),
                a,
            }
        })
        .collect();

    Ok(Some(Pixmap::from_parts_with_opacity(
        pixels,
        width,
        height,
        may_have_opacities,
    )))
}

fn image_bilinear(rendering: ImageRendering) -> bool {
    !matches!(
        rendering,
        ImageRendering::OptimizeSpeed | ImageRendering::CrispEdges | ImageRendering::Pixelated
    )
}

/// Flatten all paths in a group into a single BezPath (for clip paths).
fn flatten_group_to_path(g: &Group) -> BezPath {
    let mut bp = BezPath::new();
    for child in g.children() {
        match child {
            Node::Path(p) => {
                for seg in p.data().segments() {
                    match seg {
                        PathSegment::MoveTo(pt) => bp.move_to((pt.x as f64, pt.y as f64)),
                        PathSegment::LineTo(pt) => bp.line_to((pt.x as f64, pt.y as f64)),
                        PathSegment::QuadTo(p1, p2) => {
                            bp.quad_to((p1.x as f64, p1.y as f64), (p2.x as f64, p2.y as f64));
                        }
                        PathSegment::CubicTo(p1, p2, p3) => {
                            bp.curve_to(
                                (p1.x as f64, p1.y as f64),
                                (p2.x as f64, p2.y as f64),
                                (p3.x as f64, p3.y as f64),
                            );
                        }
                        PathSegment::Close => bp.close_path(),
                    }
                }
            }
            Node::Group(group) => {
                let sub = flatten_group_to_path(group);
                bp.extend(sub.iter());
            }
            Node::Image(_) | Node::Text(_) => {}
        }
    }
    bp
}

fn convert_transform(t: &usvg::Transform) -> Affine {
    Affine::new([
        t.sx as f64,
        t.ky as f64,
        t.kx as f64,
        t.sy as f64,
        t.tx as f64,
        t.ty as f64,
    ])
}

fn convert_path(p: &usvg::Path) -> BezPath {
    let mut bp = BezPath::new();
    for seg in p.data().segments() {
        match seg {
            PathSegment::MoveTo(pt) => bp.move_to((pt.x as f64, pt.y as f64)),
            PathSegment::LineTo(pt) => bp.line_to((pt.x as f64, pt.y as f64)),
            PathSegment::QuadTo(p1, p2) => {
                bp.quad_to((p1.x as f64, p1.y as f64), (p2.x as f64, p2.y as f64));
            }
            PathSegment::CubicTo(p1, p2, p3) => {
                bp.curve_to(
                    (p1.x as f64, p1.y as f64),
                    (p2.x as f64, p2.y as f64),
                    (p3.x as f64, p3.y as f64),
                );
            }
            PathSegment::Close => bp.close_path(),
        }
    }
    bp
}

fn usvg_paint_to_color(paint: &usvg::Paint, opacity: usvg::Opacity) -> Color {
    match paint {
        usvg::Paint::Color(c) => {
            Color::from_rgba8(c.red, c.green, c.blue, (opacity.get() * 255.0) as u8)
        }
        // For gradients/patterns, fall back to a visible grey.
        _ => Color::from_rgba8(128, 128, 128, (opacity.get() * 255.0) as u8),
    }
}

// ── BenchScene impl ──────────────────────────────────────────────────────────

impl BenchScene for SvgScene {
    fn scene_id(&self) -> SceneId {
        SceneId::Svg
    }

    fn name(&self) -> &str {
        "Vector Graphics"
    }

    fn params(&self) -> Vec<Param> {
        vec![Param {
            id: ParamId::SvgAsset,
            label: "SVG Asset",
            kind: ParamKind::Select(
                SVG_ASSETS
                    .iter()
                    .enumerate()
                    .map(|(i, desc)| (desc.name, i as f64))
                    .collect(),
            ),
            value: self.selected as f64,
        }]
    }

    fn set_param(&mut self, param: ParamId, value: f64) {
        if param == ParamId::SvgAsset {
            let idx = value as usize;
            if idx < SVG_ASSETS.len() {
                self.selected = idx;
            }
        }
    }

    fn is_ready(&mut self) -> bool {
        self.selected_asset().is_some()
    }

    fn render(
        &mut self,
        backend: &mut dyn Backend,
        resources: &mut ResourceStore,
        width: u32,
        height: u32,
        _time: f64,
        view: Affine,
    ) {
        let Some(asset) = self.selected_asset() else {
            return;
        };

        // Scale to fit viewport, center.
        let s = (width as f64 / asset.width).min(height as f64 / asset.height);
        let tx = (width as f64 - asset.width * s) / 2.0;
        let ty = (height as f64 - asset.height * s) / 2.0;
        let base = view * Affine::translate((tx, ty)) * Affine::scale(s);

        for cmd in &asset.commands {
            match cmd {
                DrawCmd::Fill {
                    path,
                    transform,
                    color,
                } => {
                    backend.set_transform(base * *transform);
                    backend.set_paint((*color).into());
                    backend.fill_path(path);
                }
                DrawCmd::Stroke {
                    path,
                    transform,
                    color,
                    width,
                } => {
                    backend.set_transform(base * *transform);
                    backend.set_paint((*color).into());
                    backend.set_stroke(Stroke::new(*width));
                    backend.stroke_path(path);
                }
                DrawCmd::PushClip { path, transform } => {
                    backend.set_transform(base * *transform);
                    backend.push_clip_path(path);
                }
                DrawCmd::Image {
                    image_idx,
                    transform,
                    width,
                    height,
                    bilinear,
                } => {
                    let Some(image) = asset.images.get(*image_idx) else {
                        continue;
                    };
                    let source = resources.get_or_upload_image(
                        SceneId::Svg,
                        SVG_IMAGE_EPOCH,
                        image.cache_key,
                        backend,
                        || image.pixmap.clone(),
                    );
                    let image = PaintImage {
                        image: source,
                        sampler: ImageSampler {
                            x_extend: Extend::Pad,
                            y_extend: Extend::Pad,
                            quality: if *bilinear {
                                ImageQuality::Medium
                            } else {
                                ImageQuality::Low
                            },
                            alpha: 1.0,
                        },
                    };
                    backend.set_transform(base * *transform);
                    backend.set_paint(image.into());
                    backend.fill_rect(&Rect::new(0.0, 0.0, *width, *height));
                }
                DrawCmd::PopClip => {
                    backend.pop_clip_path();
                }
            }
        }

        backend.reset_transform();
    }
}
