use std::sync::OnceLock;

use usvg::tiny_skia_path::PathSegment;
use usvg::{Group, Node};
use vello_common::fearless_simd::Level;
use vello_common::flatten::{FlattenCtx, Line};
use vello_common::geometry::RectU16;
use vello_common::kurbo::{Affine, BezPath, Stroke, StrokeCtx};
use vello_common::peniko::Fill;
use vello_common::strip::Strip;
use vello_common::tile::Tiles;
use vello_common::{flatten, strip};

static TIGER: OnceLock<DataItem> = OnceLock::new();

pub fn tiger() -> &'static DataItem {
    TIGER.get_or_init(|| {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/Ghostscript_Tiger.svg"
        ));
        DataItem::from_svg("Ghostscript_Tiger", bytes)
    })
}

#[derive(Clone, Debug)]
pub struct DataItem {
    pub name: String,
    pub fills: Vec<FilledPath>,
    pub strokes: Vec<StrokedPath>,
    pub width: u16,
    pub height: u16,
}

impl DataItem {
    fn from_svg(name: &str, bytes: &[u8]) -> Self {
        let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).unwrap();
        let mut ctx = ConversionContext::new();
        convert(&mut ctx, tree.root());

        Self {
            name: name.to_owned(),
            fills: ctx.fills,
            strokes: ctx.strokes,
            width: tree.size().width() as u16,
            height: tree.size().height() as u16,
        }
    }

    pub fn lines(&self) -> Vec<Line> {
        let mut line_buf = vec![];
        let mut temp_buf = vec![];

        for path in &self.fills {
            flatten::fill(
                Level::new(),
                &path.path,
                path.transform,
                &mut temp_buf,
                &mut FlattenCtx::default(),
                RectU16::new(0, 0, self.width, self.height),
            );
            line_buf.extend(&temp_buf);
        }

        for path in &self.strokes {
            let stroke = Stroke {
                width: f64::from(path.stroke_width),
                ..Default::default()
            };
            flatten::stroke(
                Level::new(),
                &path.path,
                &stroke,
                path.transform,
                &mut temp_buf,
                &mut FlattenCtx::default(),
                &mut StrokeCtx::default(),
                RectU16::new(0, 0, self.width, self.height),
            );
            line_buf.extend(&temp_buf);
        }

        line_buf
    }

    pub fn expanded_strokes(&self) -> Vec<BezPath> {
        let mut paths = vec![];
        let mut stroke_ctx = StrokeCtx::default();

        for path in &self.strokes {
            let stroke = Stroke {
                width: f64::from(path.stroke_width),
                ..Default::default()
            };
            flatten::expand_stroke(path.path.iter(), &stroke, 0.25, &mut stroke_ctx);
            paths.push(stroke_ctx.output().clone());
        }

        paths
    }

    pub fn sorted_tiles(&self) -> Tiles {
        let level = Level::new();
        let lines = self.lines();
        let mut tiles = Tiles::new(level, self.width, self.height);
        tiles.make_tiles_analytic_aa(level, &lines, self.width, self.height);
        tiles.sort_tiles();
        tiles
    }

    #[allow(dead_code)]
    pub fn strips(&self) -> (Vec<u8>, Vec<Strip>) {
        let mut strip_buf = vec![];
        let mut alpha_buf = vec![];
        let lines = self.lines();
        let tiles = self.sorted_tiles();

        strip::render(
            Level::baseline(),
            &tiles,
            &mut strip_buf,
            &mut alpha_buf,
            Fill::NonZero,
            None,
            &lines,
        );
        (alpha_buf, strip_buf)
    }
}

#[derive(Clone, Debug)]
pub struct FilledPath {
    pub path: BezPath,
    pub transform: Affine,
}

#[derive(Clone, Debug)]
pub struct StrokedPath {
    pub path: BezPath,
    pub transform: Affine,
    pub stroke_width: f32,
}

#[derive(Debug)]
struct ConversionContext {
    stack: Vec<Affine>,
    fills: Vec<FilledPath>,
    strokes: Vec<StrokedPath>,
}

impl ConversionContext {
    fn new() -> Self {
        Self {
            stack: vec![],
            fills: vec![],
            strokes: vec![],
        }
    }

    fn push(&mut self, transform: Affine) {
        self.stack
            .push(*self.stack.last().unwrap_or(&Affine::IDENTITY) * transform);
    }

    fn transform(&self) -> Affine {
        *self.stack.last().unwrap_or(&Affine::IDENTITY)
    }
}

fn convert(ctx: &mut ConversionContext, group: &Group) {
    ctx.push(convert_transform(&group.transform()));

    for child in group.children() {
        match child {
            Node::Group(group) => convert(ctx, group),
            Node::Path(path) => {
                let converted = convert_path_data(path);
                if path.fill().is_some() {
                    ctx.fills.push(FilledPath {
                        path: converted.clone(),
                        transform: ctx.transform(),
                    });
                }
                if let Some(stroke) = path.stroke() {
                    ctx.strokes.push(StrokedPath {
                        path: converted,
                        transform: ctx.transform(),
                        stroke_width: stroke.width().get(),
                    });
                }
            }
            Node::Image(_) | Node::Text(_) => {}
        }
    }

    ctx.stack.pop();
}

fn convert_transform(transform: &usvg::Transform) -> Affine {
    Affine::new([
        f64::from(transform.sx),
        f64::from(transform.ky),
        f64::from(transform.kx),
        f64::from(transform.sy),
        f64::from(transform.tx),
        f64::from(transform.ty),
    ])
}

fn convert_path_data(path: &usvg::Path) -> BezPath {
    let mut bez_path = BezPath::new();
    for element in path.data().segments() {
        match element {
            PathSegment::MoveTo(point) => bez_path.move_to((point.x, point.y)),
            PathSegment::LineTo(point) => bez_path.line_to((point.x, point.y)),
            PathSegment::QuadTo(p1, p2) => bez_path.quad_to((p1.x, p1.y), (p2.x, p2.y)),
            PathSegment::CubicTo(p1, p2, p3) => {
                bez_path.curve_to((p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y));
            }
            PathSegment::Close => bez_path.close_path(),
        }
    }
    bez_path
}
