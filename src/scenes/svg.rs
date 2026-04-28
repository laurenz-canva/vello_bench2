//! Vector graphics (SVG) benchmark scene using usvg for proper parsing.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamId, ParamKind, SceneId};
use crate::backend::{Backend, Pixmap};
use crate::resource_store::ResourceStore;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use usvg::tiny_skia_path::PathSegment;
use usvg::{Group, ImageRendering, Node};
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::Image;
use vello_common::peniko::{
    Color, Extend, ImageQuality, ImageSampler, color::PremulRgba8,
};

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
    Image {
        raster_index: usize,
        transform: Affine,
        bilinear: bool,
    },
    PushClip {
        path: BezPath,
        transform: Affine,
    },
    PopClip,
}

#[derive(Clone)]
struct SvgRasterImage {
    cache_key: u64,
    pixmap: Pixmap,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum SvgRasterFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct RasterImageKey {
    format: SvgRasterFormat,
    data: Arc<Vec<u8>>,
}

/// A pre-parsed SVG asset ready for rendering.
struct SvgAsset {
    name: &'static str,
    commands: Vec<DrawCmd>,
    raster_images: Vec<SvgRasterImage>,
    width: f64,
    height: f64,
}

/// Benchmark scene that renders one of several SVG assets.
pub struct SvgScene {
    assets: Vec<SvgAsset>,
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
    /// Create a new SVG scene with all bundled assets.
    pub fn new() -> Self {
        let load = |name: &'static str, data: &[u8]| {
            let tree = usvg::Tree::from_data(data, &usvg::Options::default())
                .unwrap_or_else(|e| panic!("Failed to parse {name}: {e}"));
            let mut commands = Vec::new();
            let mut raster_images = Vec::new();
            let mut raster_cache = HashMap::new();
            convert_group(
                &mut commands,
                &mut raster_images,
                &mut raster_cache,
                tree.root(),
                Affine::IDENTITY,
            );
            SvgAsset {
                name,
                commands,
                raster_images,
                width: tree.size().width() as f64,
                height: tree.size().height() as f64,
            }
        };
        let assets = vec![
            load(
                "Ghostscript Tiger",
                include_bytes!("../../assets/Ghostscript_Tiger.svg"),
            ),
            load(
                "Coat of Arms",
                include_bytes!("../../assets/coat_of_arms.svg"),
            ),
            load("Heraldry", include_bytes!("../../assets/heraldry.svg")),
            load("Design 1", include_bytes!("../../assets/design1.svg")),
            load("Design 2", include_bytes!("../../assets/design2.svg")),
        ];
        Self {
            assets,
            selected: 0,
        }
    }
}

// ── usvg → draw command conversion ──────────────────────────────────────────

fn convert_group(
    commands: &mut Vec<DrawCmd>,
    raster_images: &mut Vec<SvgRasterImage>,
    raster_cache: &mut HashMap<RasterImageKey, usize>,
    g: &Group,
    parent_transform: Affine,
) {
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
                convert_group(commands, raster_images, raster_cache, group, transform)
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
                convert_image(commands, raster_images, raster_cache, image, transform);
            }
            Node::Text(_) => {}
        }
    }

    if has_clip {
        commands.push(DrawCmd::PopClip);
    }
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

fn convert_image(
    commands: &mut Vec<DrawCmd>,
    raster_images: &mut Vec<SvgRasterImage>,
    raster_cache: &mut HashMap<RasterImageKey, usize>,
    image: &usvg::Image,
    transform: Affine,
) {
    if !image.is_visible() {
        return;
    }

    match image.kind() {
        usvg::ImageKind::SVG(tree) => {
            convert_group(commands, raster_images, raster_cache, tree.root(), transform);
        }
        usvg::ImageKind::JPEG(data) => {
            push_raster_image(
                commands,
                raster_images,
                raster_cache,
                SvgRasterFormat::Jpeg,
                data,
                transform,
                prefers_bilinear(image.rendering_mode()),
            );
        }
        usvg::ImageKind::PNG(data) => {
            push_raster_image(
                commands,
                raster_images,
                raster_cache,
                SvgRasterFormat::Png,
                data,
                transform,
                prefers_bilinear(image.rendering_mode()),
            );
        }
        usvg::ImageKind::GIF(data) => {
            push_raster_image(
                commands,
                raster_images,
                raster_cache,
                SvgRasterFormat::Gif,
                data,
                transform,
                prefers_bilinear(image.rendering_mode()),
            );
        }
        usvg::ImageKind::WEBP(data) => {
            push_raster_image(
                commands,
                raster_images,
                raster_cache,
                SvgRasterFormat::Webp,
                data,
                transform,
                prefers_bilinear(image.rendering_mode()),
            );
        }
    }
}

fn push_raster_image(
    commands: &mut Vec<DrawCmd>,
    raster_images: &mut Vec<SvgRasterImage>,
    raster_cache: &mut HashMap<RasterImageKey, usize>,
    format: SvgRasterFormat,
    data: &Arc<Vec<u8>>,
    transform: Affine,
    bilinear: bool,
) {
    let key = RasterImageKey {
        format,
        data: Arc::clone(data),
    };
    let raster_index = if let Some(&idx) = raster_cache.get(&key) {
        idx
    } else {
        let pixmap = match decode_svg_raster_image(format, data) {
            Ok(pixmap) => pixmap,
            Err(err) => {
                log::warn!("Skipping SVG image: {err}");
                return;
            }
        };
        let idx = raster_images.len();
        raster_images.push(SvgRasterImage {
            cache_key: idx as u64,
            pixmap,
        });
        raster_cache.insert(key, idx);
        idx
    };

    commands.push(DrawCmd::Image {
        raster_index,
        transform,
        bilinear,
    });
}

fn decode_svg_raster_image(format: SvgRasterFormat, data: &[u8]) -> Result<Pixmap, String> {
    match format {
        SvgRasterFormat::Png => Pixmap::from_png(Cursor::new(data))
            .map_err(|err| format!("failed to decode embedded PNG: {err}")),
        SvgRasterFormat::Jpeg | SvgRasterFormat::Gif | SvgRasterFormat::Webp => {
            decode_image_with_image_crate(data)
        }
    }
}

fn decode_image_with_image_crate(data: &[u8]) -> Result<Pixmap, String> {
    let image = image::load_from_memory(data)
        .map_err(|err| format!("failed to decode embedded raster image: {err}"))?
        .into_rgba8();
    let width: u16 = image
        .width()
        .try_into()
        .map_err(|_| "embedded image width exceeds u16".to_string())?;
    let height: u16 = image
        .height()
        .try_into()
        .map_err(|_| "embedded image height exceeds u16".to_string())?;

    let mut may_have_opacities = false;
    let pixels = image
        .into_vec()
        .chunks_exact(4)
        .map(|rgba| {
            let alpha = u16::from(rgba[3]);
            may_have_opacities |= alpha != 255;
            let premultiply = |component| ((alpha * u16::from(component)) / 255) as u8;
            PremulRgba8 {
                r: premultiply(rgba[0]),
                g: premultiply(rgba[1]),
                b: premultiply(rgba[2]),
                a: alpha as u8,
            }
        })
        .collect();

    Ok(Pixmap::from_parts_with_opacity(
        pixels,
        width,
        height,
        may_have_opacities,
    ))
}

fn prefers_bilinear(rendering: ImageRendering) -> bool {
    let _ = rendering;
    true
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
                self.assets
                    .iter()
                    .enumerate()
                    .map(|(i, a)| (a.name, i as f64))
                    .collect(),
            ),
            value: self.selected as f64,
        }]
    }

    fn set_param(&mut self, param: ParamId, value: f64) {
        if param == ParamId::SvgAsset {
            let idx = value as usize;
            if idx < self.assets.len() {
                self.selected = idx;
            }
        }
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
        let asset = &self.assets[self.selected];

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
                DrawCmd::Image {
                    raster_index,
                    transform,
                    bilinear,
                } => {
                    let raster = &asset.raster_images[*raster_index];
                    let source = resources.get_or_upload_image(
                        SceneId::Svg,
                        self.selected as u64,
                        raster.cache_key,
                        backend,
                        || raster.pixmap.clone(),
                    );
                    backend.set_transform(base * *transform);
                    backend.reset_paint_transform();
                    backend.set_paint(
                        Image {
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
                        }
                        .into(),
                    );
                    backend.fill_rect(&Rect::new(
                        0.0,
                        0.0,
                        raster.pixmap.width() as f64,
                        raster.pixmap.height() as f64,
                    ));
                }
                DrawCmd::PushClip { path, transform } => {
                    backend.set_transform(base * *transform);
                    backend.push_clip_path(path);
                }
                DrawCmd::PopClip => {
                    backend.pop_clip_path();
                }
            }
        }

        backend.reset_transform();
    }
}
