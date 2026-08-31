use std::hint::black_box;
use std::io::Cursor;
use std::sync::Arc;

use smallvec::smallvec;
use vello_common::color::DynamicColor;
use vello_common::color::palette::css::{BLUE, GREEN, RED, ROYAL_BLUE, YELLOW};
use vello_common::encode::{EncodeExt, EncodedPaint};
use vello_common::fearless_simd::{Simd, dispatch};
use vello_common::geometry::RectU16;
use vello_common::kurbo::{Affine, Point};
use vello_common::paint::{Image, ImageSource, NoOpImageResolver, Paint, PremulColor};
use vello_common::peniko::{
    BlendMode, ColorStop, ColorStops, Compose, Extend, Gradient, GradientKind, ImageQuality,
    ImageSampler, Mix,
};
use vello_common::pixmap::{Pixmap, PixmapMut};
use vello_common::tile::Tile;
use vello_cpu::Level;
use vello_cpu::fine::{Fine, FineResources, PaintFillAttrs, Span, U8Kernel};
use vello_cpu::peniko::{LinearGradientPosition, RadialGradientPosition, SweepGradientPosition};
use vello_cpu::region::Region;

use crate::cases::BenchCase;

const BENCH_WIDTH: u16 = 256;
pub fn core_cases() -> Vec<Box<dyn BenchCase>> {
    let mut cases: Vec<Box<dyn BenchCase>> = vec![];

    for (name, alpha, width) in [
        ("opaque_short", 1.0, 32),
        ("opaque_long", 1.0, 256),
        ("transparent_short", 0.3, 32),
        ("transparent_long", 0.3, 256),
    ] {
        let color = if alpha == 1.0 {
            ROYAL_BLUE
        } else {
            ROYAL_BLUE.with_alpha(alpha)
        };
        cases.push(Box::new(FineCase::fill(
            format!("fine/fill/{name}_u8"),
            Paint::Solid(PremulColor::from_alpha_color(color)),
            vec![],
            width,
            None,
        )));
    }

    let mut random = 0xA341_316C_u32;
    let alphas = Arc::new(
        (0..usize::from(BENCH_WIDTH) * usize::from(Tile::HEIGHT))
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                random as u8
            })
            .collect::<Vec<u8>>(),
    );
    for (name, width) in [
        ("solid_single", Tile::WIDTH),
        ("solid_short", 8),
        ("solid_medium", 16),
        ("solid_long", 64),
    ] {
        cases.push(Box::new(FineCase::fill(
            format!("fine/strip/{name}_u8"),
            Paint::Solid(PremulColor::from_alpha_color(ROYAL_BLUE)),
            vec![],
            width,
            Some(Arc::clone(&alphas)),
        )));
    }

    cases.push(Box::new(FineCase::simple(
        "fine/pack/pack_block_u8",
        FineOperation::Pack,
    )));
    cases.push(Box::new(FineCase::simple(
        "fine/pack/unpack_block_u8",
        FineOperation::Unpack,
    )));

    cases.push(Box::new(gradient_case(
        "fine/gradient/linear/opaque_u8",
        LinearGradientPosition {
            start: Point::new(128.0, 128.0),
            end: Point::new(134.0, 134.0),
        }
        .into(),
    )));
    cases.push(Box::new(gradient_case(
        "fine/gradient/radial/opaque_u8",
        RadialGradientPosition {
            start_center: Point::new(f64::from(BENCH_WIDTH) / 2.0, f64::from(Tile::HEIGHT) / 2.0),
            start_radius: 25.0,
            end_center: Point::new(f64::from(BENCH_WIDTH) / 2.0, f64::from(Tile::HEIGHT) / 2.0),
            end_radius: 75.0,
        }
        .into(),
    )));
    cases.push(Box::new(gradient_case(
        "fine/gradient/sweep/opaque_u8",
        SweepGradientPosition {
            center: Point::new(f64::from(BENCH_WIDTH) / 2.0, f64::from(Tile::HEIGHT) / 2.0),
            start_angle: 70.0_f32.to_radians(),
            end_angle: 250.0_f32.to_radians(),
        }
        .into(),
    )));

    cases.push(Box::new(image_case(
        "fine/image/transform/scale_u8",
        ImageQuality::Low,
    )));
    cases.push(Box::new(image_case(
        "fine/image/quality/low_u8",
        ImageQuality::Low,
    )));
    cases.push(Box::new(image_case(
        "fine/image/quality/medium_u8",
        ImageQuality::Medium,
    )));

    cases
}

struct FineCase {
    name: String,
    operation: FineOperation,
}

enum FineOperation {
    Fill {
        paint: Paint,
        encoded_paints: Vec<EncodedPaint>,
        width: u16,
        alphas: Option<Arc<Vec<u8>>>,
    },
    Pack,
    Unpack,
}

impl FineCase {
    fn simple(name: &str, operation: FineOperation) -> Self {
        Self {
            name: name.to_owned(),
            operation,
        }
    }

    fn fill(
        name: String,
        paint: Paint,
        encoded_paints: Vec<EncodedPaint>,
        width: u16,
        alphas: Option<Arc<Vec<u8>>>,
    ) -> Self {
        Self {
            name,
            operation: FineOperation::Fill {
                paint,
                encoded_paints,
                width,
                alphas,
            },
        }
    }
}

impl BenchCase for FineCase {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, iterations: u32) {
        dispatch!(Level::new(), simd => run_with_simd(simd, &self.operation, iterations));
    }
}

#[inline(always)]
fn run_with_simd<S: Simd>(simd: S, operation: &FineOperation, iterations: u32) {
    let mut fine = Fine::<S, U8Kernel>::new(simd, BENCH_WIDTH);
    match operation {
        FineOperation::Fill {
            paint,
            encoded_paints,
            width,
            alphas,
        } => {
            let attrs = PaintFillAttrs {
                paint: paint.clone(),
                blend_mode: default_blend(),
                mask: None,
                draw_id: 1,
                thread_idx: 0,
                origin: (0, 0),
            };
            for _ in 0..iterations {
                fine.paint_fill(
                    Span::new(0, *width),
                    &attrs,
                    FineResources {
                        alpha_buffers: &[],
                        encoded_paints,
                        filter_paints: &[],
                        image_resolver: &NoOpImageResolver,
                    },
                    alphas.as_deref().map(Vec::as_slice),
                );
                black_box(&fine);
            }
        }
        FineOperation::Pack => {
            let mut buffer = vec![0; usize::from(BENCH_WIDTH) * usize::from(Tile::HEIGHT) * 4];
            for _ in 0..iterations {
                let mut pixmap = PixmapMut::new(BENCH_WIDTH, Tile::HEIGHT, &mut buffer).unwrap();
                let mut region =
                    Region::new(&mut pixmap, RectU16::new(0, 0, BENCH_WIDTH, Tile::HEIGHT));
                fine.pack(&mut region);
                black_box(&buffer);
            }
        }
        FineOperation::Unpack => {
            let mut buffer = vec![0; usize::from(BENCH_WIDTH) * usize::from(Tile::HEIGHT) * 4];
            for _ in 0..iterations {
                let mut pixmap = PixmapMut::new(BENCH_WIDTH, Tile::HEIGHT, &mut buffer).unwrap();
                let mut region =
                    Region::new(&mut pixmap, RectU16::new(0, 0, BENCH_WIDTH, Tile::HEIGHT));
                fine.unpack(0, &mut region);
                black_box(&fine);
            }
        }
    }
}

fn gradient_case(name: &str, kind: GradientKind) -> FineCase {
    let stops = ColorStops(smallvec![
        ColorStop {
            offset: 0.0,
            color: DynamicColor::from_alpha_color(BLUE),
        },
        ColorStop {
            offset: 0.33,
            color: DynamicColor::from_alpha_color(GREEN),
        },
        ColorStop {
            offset: 0.66,
            color: DynamicColor::from_alpha_color(RED),
        },
        ColorStop {
            offset: 1.0,
            color: DynamicColor::from_alpha_color(YELLOW),
        },
    ]);
    let gradient = Gradient {
        kind,
        stops,
        extend: Extend::Pad,
        ..Default::default()
    };
    let mut paints = vec![];
    let paint = gradient.encode_into(&mut paints, Affine::IDENTITY, None);
    FineCase::fill(name.to_owned(), paint, paints, BENCH_WIDTH, None)
}

fn image_case(name: &str, quality: ImageQuality) -> FineCase {
    let data = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vello_2/vello_tests/snapshots/big_colr.png"
    ));
    let pixmap = Pixmap::from_png(Cursor::new(data)).unwrap();
    let image = Image {
        image: ImageSource::Pixmap(Arc::new(pixmap)),
        sampler: ImageSampler {
            x_extend: Extend::Pad,
            y_extend: Extend::Pad,
            quality,
            alpha: 1.0,
        },
    };
    let mut paints = vec![];
    let paint = image.encode_into(&mut paints, Affine::scale(3.0), None);
    FineCase::fill(name.to_owned(), paint, paints, BENCH_WIDTH, None)
}

fn default_blend() -> BlendMode {
    BlendMode::new(Mix::Normal, Compose::SrcOver)
}
