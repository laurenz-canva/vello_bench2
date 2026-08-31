use std::hint::black_box;
use std::sync::Arc;

use vello_common::fearless_simd::Level;
use vello_common::flatten::{self, FlattenCtx, Line};
use vello_common::geometry::RectU16;
use vello_common::kurbo::{Affine, Rect, Shape, Stroke, StrokeCtx};
use vello_common::peniko::{Fill, ImageAlphaType};
use vello_common::pixmap::{PixelMetadata, Pixmap};
use vello_common::strip_generator::{StripGenerator, StripStorage};
use vello_common::tile::Tiles;

use crate::data::{DataItem, tiger};
use crate::now;

pub trait BenchCase {
    fn name(&self) -> &str;
    fn run(&mut self, iterations: u32);

    fn measure(&mut self, iterations: u32) -> f64 {
        let start = now();
        self.run(iterations);
        now() - start
    }
}

pub fn core_cases() -> Vec<Box<dyn BenchCase>> {
    let tiger = Arc::new(tiger().clone());
    let lines = Arc::new(tiger.lines());
    let tiles = Arc::new(tiger.sorted_tiles());

    let mut cases: Vec<Box<dyn BenchCase>> = vec![
        Box::new(PixmapCase::new(PixmapOperation::Premultiply, false)),
        Box::new(PixmapCase::new(PixmapOperation::Premultiply, true)),
        Box::new(PixmapCase::new(PixmapOperation::Unpremultiply, false)),
        Box::new(PixmapCase::new(PixmapOperation::Unpremultiply, true)),
        Box::new(TileCase {
            name: format!("tile_aaa/{}", tiger.name),
            lines: Arc::clone(&lines),
            width: tiger.width,
            height: tiger.height,
        }),
        Box::new(StripCase {
            name: format!("render_strips/{}", tiger.name),
            lines,
            tiles,
            strip_buf: vec![],
            alpha_buf: vec![],
        }),
        Box::new(RectCase::new(false)),
        Box::new(RectCase::new(true)),
        Box::new(FlattenCase::new(Arc::clone(&tiger))),
        Box::new(StrokesCase::new(tiger)),
    ];
    cases.extend(crate::fine::core_cases());
    cases
}

#[derive(Clone, Copy)]
enum PixmapOperation {
    Premultiply,
    Unpremultiply,
}

struct PixmapCase {
    name: String,
    operation: PixmapOperation,
    bytes: Vec<u8>,
    pixmap: Pixmap,
}

impl PixmapCase {
    const WIDTH: u16 = 1920;
    const HEIGHT: u16 = 1080;

    fn new(operation: PixmapOperation, translucent: bool) -> Self {
        let pixel = if translucent {
            [0, 0, 255, 128]
        } else {
            [0, 0, 255, 255]
        };
        let label = if translucent { "translucent" } else { "opaque" };
        let bytes = pixel.repeat(usize::from(Self::WIDTH) * usize::from(Self::HEIGHT));
        let pixmap = Pixmap::from_parts(
            bytes.clone(),
            Self::WIDTH,
            Self::HEIGHT,
            PixelMetadata::new(ImageAlphaType::Alpha, true),
        );
        let operation_name = match operation {
            PixmapOperation::Premultiply => "premultiply",
            PixmapOperation::Unpremultiply => "unpremultiply",
        };
        Self {
            name: format!("pixmap/{operation_name}/{label}"),
            operation,
            bytes,
            pixmap,
        }
    }
}

impl BenchCase for PixmapCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            match self.operation {
                PixmapOperation::Premultiply => {
                    let bytes = self.bytes.clone();
                    black_box(Pixmap::from_parts(
                        bytes,
                        Self::WIDTH,
                        Self::HEIGHT,
                        PixelMetadata::new(ImageAlphaType::Alpha, true),
                    ));
                }
                PixmapOperation::Unpremultiply => {
                    black_box(self.pixmap.clone().take_unpremultiplied());
                }
            }
        }
    }

    // Criterion's iter_batched excludes the large input clone. Keep the same boundary here.
    fn measure(&mut self, iterations: u32) -> f64 {
        let mut elapsed = 0.0;
        for _ in 0..iterations {
            match self.operation {
                PixmapOperation::Premultiply => {
                    let bytes = self.bytes.clone();
                    let start = now();
                    black_box(Pixmap::from_parts(
                        bytes,
                        Self::WIDTH,
                        Self::HEIGHT,
                        PixelMetadata::new(ImageAlphaType::Alpha, true),
                    ));
                    elapsed += now() - start;
                }
                PixmapOperation::Unpremultiply => {
                    let pixmap = self.pixmap.clone();
                    let start = now();
                    black_box(pixmap.take_unpremultiplied());
                    elapsed += now() - start;
                }
            }
        }
        elapsed
    }
}

struct TileCase {
    name: String,
    lines: Arc<Vec<Line>>,
    width: u16,
    height: u16,
}

impl BenchCase for TileCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        let level = Level::new();
        for _ in 0..iterations {
            let mut tiler = Tiles::new(level, self.width, self.height);
            tiler.make_tiles_analytic_aa(level, &self.lines, self.width, self.height);
            black_box(tiler);
        }
    }
}

struct StripCase {
    name: String,
    lines: Arc<Vec<Line>>,
    tiles: Arc<Tiles>,
    strip_buf: Vec<vello_common::strip::Strip>,
    alpha_buf: Vec<u8>,
}

impl BenchCase for StripCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        let level = Level::new();
        for _ in 0..iterations {
            self.strip_buf.clear();
            self.alpha_buf.clear();
            vello_common::strip::render(
                level,
                &self.tiles,
                &mut self.strip_buf,
                &mut self.alpha_buf,
                Fill::NonZero,
                None,
                &self.lines,
            );
            black_box((&self.strip_buf, &self.alpha_buf));
        }
    }
}

struct RectCase {
    name: String,
    fast: bool,
    rect: Rect,
    generator: StripGenerator,
    storage: StripStorage,
}

impl RectCase {
    fn new(fast: bool) -> Self {
        Self {
            name: format!(
                "render_rect/{}",
                if fast {
                    "14x14_via_rect"
                } else {
                    "14x14_via_path"
                }
            ),
            fast,
            rect: Rect::new(10.0, 10.0, 24.0, 24.0),
            generator: StripGenerator::new(100, 100, Level::new()),
            storage: StripStorage::default(),
        }
    }
}

impl BenchCase for RectCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            self.storage.clear();
            if self.fast {
                self.generator
                    .generate_filled_rect_fast(&self.rect, &mut self.storage, None);
            } else {
                self.generator.generate_filled_path(
                    self.rect.to_path(0.1),
                    Fill::NonZero,
                    Affine::IDENTITY,
                    None,
                    &mut self.storage,
                    None,
                );
            }
            self.generator.reset(100, 100);
            black_box(&self.storage);
        }
    }
}

struct FlattenCase {
    name: String,
    item: Arc<DataItem>,
    expanded_strokes: Vec<vello_common::kurbo::BezPath>,
    line_buf: Vec<Line>,
    temp_buf: Vec<Line>,
    flatten_ctx: FlattenCtx,
}

impl FlattenCase {
    fn new(item: Arc<DataItem>) -> Self {
        Self {
            name: format!("flatten/{}", item.name),
            expanded_strokes: item.expanded_strokes(),
            item,
            line_buf: vec![],
            temp_buf: vec![],
            flatten_ctx: FlattenCtx::default(),
        }
    }
}

impl BenchCase for FlattenCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        let bounds = RectU16::new(0, 0, self.item.width, self.item.height);
        for _ in 0..iterations {
            self.line_buf.clear();
            for path in &self.item.fills {
                flatten::fill(
                    Level::new(),
                    &path.path,
                    path.transform,
                    &mut self.temp_buf,
                    &mut self.flatten_ctx,
                    bounds,
                );
                self.line_buf.extend(&self.temp_buf);
            }
            for stroke in &self.expanded_strokes {
                flatten::fill(
                    Level::new(),
                    stroke,
                    Affine::IDENTITY,
                    &mut self.temp_buf,
                    &mut self.flatten_ctx,
                    bounds,
                );
                self.line_buf.extend(&self.temp_buf);
            }
            black_box(&self.line_buf);
        }
    }
}

struct StrokesCase {
    name: String,
    item: Arc<DataItem>,
    stroke_ctx: StrokeCtx,
}

impl StrokesCase {
    fn new(item: Arc<DataItem>) -> Self {
        Self {
            name: format!("strokes/{}", item.name),
            item,
            stroke_ctx: StrokeCtx::default(),
        }
    }
}

impl BenchCase for StrokesCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            let mut paths = vec![];
            for path in &self.item.strokes {
                let stroke = Stroke {
                    width: f64::from(path.stroke_width),
                    ..Default::default()
                };
                flatten::expand_stroke(path.path.iter(), &stroke, 0.25, &mut self.stroke_ctx);
                paths.push(self.stroke_ctx.output().clone());
            }
            black_box(paths);
        }
    }
}
