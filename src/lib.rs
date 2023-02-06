use egui::*;
use lyon::lyon_tessellation::geometry_builder::*;
use lyon::lyon_tessellation::*;
use lyon::math::Point;
use lyon::path::PathEvent;
use std::rc::Rc;
use std::sync::Arc;

/// ???
#[cfg(feature = "cached")]
macro_rules! bytes {
    ($t:expr, $T:ty) => {
        unsafe { std::mem::transmute::<$T, [u8; std::mem::size_of::<$T>()]>($t) }
    };
}

#[derive(Clone, Copy)]
pub enum FitMode {
    None,
    Size(Vec2),
    Factor(f32),
    Cover,
    Contain(Margin),
}

pub struct Svg {
    tree: Rc<usvg::Tree>,
    #[cfg(feature = "cached")]
    key: u64,
    color_func: Option<Arc<dyn Fn(&mut Color32)>>,
    tolerance: f32,
    scale_tolerance: bool,
    fit_mode: FitMode,
}
#[cfg(feature = "cached")]
impl std::hash::Hash for Svg {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let Self {
            tree: _,
            key,
            color_func: _,
            tolerance,
            scale_tolerance,
            fit_mode,
        } = self;
        key.hash(state);
        bytes!(*tolerance, f32).hash(state);
        scale_tolerance.hash(state);
        match fit_mode {
            FitMode::None => 0usize.hash(state),
            FitMode::Size(s) => {
                1usize.hash(state);
                bytes!(*s, Vec2).hash(state);
            }
            FitMode::Factor(f) => {
                2usize.hash(state);
                bytes!(*f, f32).hash(state);
            }
            FitMode::Cover => 3usize.hash(state),
            FitMode::Contain(margin) => {
                4usize.hash(state);
                bytes!(*margin, Margin).hash(state);
            }
        }
    }
}
impl Svg {
    /// load a svg icon from buffer
    #[cfg_attr(feature = "cached", doc = "")]
    #[cfg_attr(feature = "cached", doc = "`cached`: cached svg tree will never drop")]
    #[cfg_attr(feature = "static_cached", doc = "")]
    #[cfg_attr(
        feature = "static_cached",
        doc = "`static_cached`: using ptr as cache key so `data` must be `'static`"
    )]
    pub fn new(
        #[cfg(not(feature = "static_cached"))] data: &[u8],
        #[cfg(feature = "static_cached")] data: &'static [u8],
    ) -> Self {
        #[cfg(feature = "puffin")]
        puffin::profile_function!();

        #[cfg(not(feature = "cached"))]
        let tree = Rc::new(usvg::Tree::from_data(data, &usvg::Options::default()).unwrap());

        #[cfg(feature = "cached")]
        let (key, tree) = {
            use egui::epaint::ahash::*;
            use std::cell::RefCell;
            use std::hash::*;

            thread_local! {
                static CACHE: RefCell<HashMap<u64, Rc<usvg::Tree>>> = Default::default();
            }
            CACHE.with(|cache| {
                let key = {
                    let mut hasher = RandomState::with_seed(0).build_hasher();

                    #[cfg(not(feature = "static_cached"))]
                    data.hash(&mut hasher);

                    #[cfg(feature = "static_cached")]
                    data.as_ptr().hash(&mut hasher);

                    hasher.finish()
                };

                (
                    key,
                    cache
                        .borrow_mut()
                        .entry(key)
                        .or_insert_with(|| {
                            Rc::new(usvg::Tree::from_data(data, &usvg::Options::default()).unwrap())
                        })
                        .clone(),
                )
            })
        };

        Svg {
            tree,
            #[cfg(feature = "cached")]
            key,
            color_func: None,
            tolerance: 1.0,
            scale_tolerance: true,
            fit_mode: FitMode::Contain(Default::default()),
        }
    }
    /// set the tessellation tolerance
    pub fn with_tolerance(mut self, tolerance: f32) -> Self {
        self.tolerance = tolerance;
        self
    }
    /// set whether the tessellation tolerance is affected by the scale
    pub fn with_scale_tolerance(mut self, scale_tolerance: bool) -> Self {
        self.scale_tolerance = scale_tolerance;
        self
    }
    /// override all elements' color
    pub fn with_color_remap(mut self, func: impl Fn(&mut Color32) + 'static) -> Self {
        self.color_func = Some(Arc::new(func));
        self
    }
    /// override all elements' color
    pub fn with_color(mut self, color: Color32) -> Self {
        self.color_func = Some(Arc::new(move |c| *c = color));
        self
    }
    /// set how the shape fits into the frame
    pub fn with_fit_mode(mut self, fit_mode: FitMode) -> Self {
        self.fit_mode = fit_mode;
        self
    }
    /// show the icon at the svg's original size
    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.svg_rect().size();
        self.show_sized(ui, size)
    }
    /// show the icon. size is based on available height of the ui
    pub fn show_justified(self, ui: &mut Ui) -> Response {
        let size = [
            ui.available_height() * self.svg_rect().aspect_ratio(),
            ui.available_height(),
        ];
        self.show_sized(ui, size)
    }
    /// show the icon at the given size
    pub fn show_sized(self, ui: &mut Ui, size: impl Into<Vec2>) -> Response {
        #[cfg(feature = "puffin")]
        puffin::profile_function!();

        let size = size.into();
        let (id, frame_rect) = ui.allocate_space(size);
        let mut inner_frame_rect = frame_rect;
        let size = match self.fit_mode {
            FitMode::None => self.svg_rect().size(),
            FitMode::Size(s) => s,
            FitMode::Factor(f) => self.svg_rect().size() * f,
            FitMode::Cover => Vec2::from(
                if frame_rect.aspect_ratio() > self.svg_rect().aspect_ratio() {
                    [
                        frame_rect.width(),
                        self.svg_rect().height() * frame_rect.width() / self.svg_rect().width(),
                    ]
                } else {
                    [
                        self.svg_rect().width() * frame_rect.height() / self.svg_rect().height(),
                        frame_rect.height(),
                    ]
                },
            ),
            FitMode::Contain(margin) => {
                inner_frame_rect.min += margin.left_top();
                inner_frame_rect.max -= margin.right_bottom();
                Vec2::from(
                    if inner_frame_rect.aspect_ratio() > self.svg_rect().aspect_ratio() {
                        [
                            self.svg_rect().width() * inner_frame_rect.height()
                                / self.svg_rect().height(),
                            inner_frame_rect.height(),
                        ]
                    } else {
                        [
                            inner_frame_rect.width(),
                            self.svg_rect().height() * inner_frame_rect.width()
                                / self.svg_rect().width(),
                        ]
                    },
                )
            }
        };
        let rect = Align2::CENTER_CENTER.align_size_within_rect(size, inner_frame_rect);

        #[cfg(not(feature = "cached"))]
        let shape = self.tessellate(rect, size / self.svg_rect().size());

        #[cfg(feature = "cached")]
        let shape = {
            use egui::util::cache::*;
            use std::hash::*;

            #[derive(Clone, Copy)]
            struct TessellateCacheKey<'l>(&'l Svg, Vec2);
            impl Hash for TessellateCacheKey<'_> {
                fn hash<H: Hasher>(&self, state: &mut H) {
                    let TessellateCacheKey(svg, size) = self;
                    svg.hash(state);
                    bytes!(*size, Vec2).hash(state);
                }
            }

            #[derive(Default)]
            struct Tessellator;
            impl ComputerMut<TessellateCacheKey<'_>, Mesh> for Tessellator {
                fn compute(&mut self, TessellateCacheKey(svg, size): TessellateCacheKey) -> Mesh {
                    svg.tessellate(
                        Rect::from_min_size(Pos2::ZERO, size),
                        size / svg.svg_rect().size(),
                    )
                }
            }

            let mut mesh = ui.memory_mut(|mem| {
                mem.caches
                    .cache::<FrameCache<_, Tessellator>>()
                    .get(TessellateCacheKey(&self, size))
            });
            mesh.translate(rect.min.to_vec2());
            if let Some(color_fonc) = self.color_func {
                mesh.vertices
                    .iter_mut()
                    .for_each(|f| color_fonc(&mut f.color));
            }
            mesh
        };

        ui.painter().with_clip_rect(frame_rect).add(shape);
        ui.interact(rect, id, Sense::hover())
    }

    fn svg_rect(&self) -> Rect {
        self.tree.view_box.rect.convert()
    }
    fn tessellate(&self, rect: Rect, scale: Vec2) -> Mesh {
        #[cfg(feature = "puffin")]
        puffin::profile_function!();

        let mut buffer = VertexBuffers::<_, u32>::new();
        self.tessellate_recursive(
            scale,
            rect,
            &mut buffer,
            &mut FillTessellator::new(),
            &mut StrokeTessellator::new(),
            &self.tree.root,
            Default::default(),
        );

        let mut mesh = Mesh::default();
        std::mem::swap(&mut buffer.vertices, &mut mesh.vertices);
        std::mem::swap(&mut buffer.indices, &mut mesh.indices);
        mesh
    }
    fn tessellate_recursive(
        &self,
        scale: Vec2,
        rect: Rect,
        buffer: &mut VertexBuffers<epaint::Vertex, u32>,
        fill_tesselator: &mut FillTessellator,
        stroke_tesselator: &mut StrokeTessellator,
        parent: &usvg::Node,
        parent_transform: usvg::Transform,
    ) {
        for node in parent.children() {
            match &*node.borrow() {
                usvg::NodeKind::Path(p) => {
                    let new_egui_vertex =
                        |point: Point, paint: &usvg::Paint, opacity: f64| -> epaint::Vertex {
                            epaint::Vertex {
                                pos: {
                                    let mut pos = Vec2::from(point.to_array());
                                    pos = {
                                        let mut transform = parent_transform;
                                        transform.append(&p.transform);
                                        let (x, y) = transform.apply(pos.x as _, pos.y as _);
                                        Vec2::new(x as _, y as _)
                                    };
                                    pos -= self.svg_rect().min.to_vec2();
                                    pos.x *= scale.x;
                                    pos.y *= scale.y;
                                    pos += rect.min.to_vec2();
                                    pos.to_pos2()
                                },
                                uv: Pos2::ZERO,
                                color: {
                                    let color = match paint {
                                        usvg::Paint::Color(c) => *c,
                                        _ => usvg::Color::black(),
                                    };
                                    let mut color = (color, opacity).convert();
                                    if let Some(func) = &self.color_func {
                                        func(&mut color);
                                    }
                                    color
                                },
                            }
                        };
                    let tolerance = if self.scale_tolerance {
                        self.tolerance / scale.max_elem()
                    } else {
                        self.tolerance
                    };
                    if let Some(fill) = &p.fill {
                        fill_tesselator
                            .tessellate(
                                p.convert(),
                                &FillOptions::tolerance(tolerance),
                                &mut BuffersBuilder::new(buffer, |f: FillVertex| {
                                    new_egui_vertex(f.position(), &fill.paint, fill.opacity.get())
                                }),
                            )
                            .unwrap();
                    }
                    if let Some(stroke) = &p.stroke {
                        stroke_tesselator
                            .tessellate(
                                p.convert(),
                                &stroke.convert().with_tolerance(tolerance),
                                &mut BuffersBuilder::new(buffer, |f: StrokeVertex| {
                                    new_egui_vertex(
                                        f.position(),
                                        &stroke.paint,
                                        stroke.opacity.get(),
                                    )
                                }),
                            )
                            .unwrap();
                    }
                }
                usvg::NodeKind::Group(g) => {
                    let mut transform = parent_transform;
                    transform.append(&g.transform);
                    self.tessellate_recursive(
                        scale,
                        rect,
                        buffer,
                        fill_tesselator,
                        stroke_tesselator,
                        &node,
                        transform,
                    )
                }
                usvg::NodeKind::Image(_) | usvg::NodeKind::Text(_) => {}
            }
        }
    }
}

// https://github.com/nical/lyon/blob/f097646635a4df9d99a51f0d81b538e3c3aa1adf/examples/wgpu_svg/src/main.rs#L677
struct PathConvIter<'a> {
    iter: usvg::PathSegmentsIter<'a>,
    prev: Point,
    first: Point,
    needs_end: bool,
    deferred: Option<PathEvent>,
}
impl<'l> Iterator for PathConvIter<'l> {
    type Item = PathEvent;
    fn next(&mut self) -> Option<PathEvent> {
        if self.deferred.is_some() {
            return self.deferred.take();
        }

        let next = self.iter.next();
        match next {
            Some(usvg::PathSegment::MoveTo { x, y }) => {
                if self.needs_end {
                    let last = self.prev;
                    let first = self.first;
                    self.needs_end = false;
                    self.prev = Point::new(x as f32, y as f32);
                    self.deferred = Some(PathEvent::Begin { at: self.prev });
                    self.first = self.prev;
                    Some(PathEvent::End {
                        last,
                        first,
                        close: false,
                    })
                } else {
                    self.first = Point::new(x as f32, y as f32);
                    self.needs_end = true;
                    Some(PathEvent::Begin { at: self.first })
                }
            }
            Some(usvg::PathSegment::LineTo { x, y }) => {
                self.needs_end = true;
                let from = self.prev;
                self.prev = Point::new(x as f32, y as f32);
                Some(PathEvent::Line {
                    from,
                    to: self.prev,
                })
            }
            Some(usvg::PathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            }) => {
                self.needs_end = true;
                let from = self.prev;
                self.prev = Point::new(x as f32, y as f32);
                Some(PathEvent::Cubic {
                    from,
                    ctrl1: Point::new(x1 as f32, y1 as f32),
                    ctrl2: Point::new(x2 as f32, y2 as f32),
                    to: self.prev,
                })
            }
            Some(usvg::PathSegment::ClosePath) => {
                self.needs_end = false;
                self.prev = self.first;
                Some(PathEvent::End {
                    last: self.prev,
                    first: self.first,
                    close: true,
                })
            }
            None => {
                if self.needs_end {
                    self.needs_end = false;
                    let last = self.prev;
                    let first = self.first;
                    Some(PathEvent::End {
                        last,
                        first,
                        close: false,
                    })
                } else {
                    None
                }
            }
        }
    }
}

trait Convert<'l, T> {
    fn convert(&'l self) -> T;
}
impl Convert<'_, StrokeOptions> for usvg::Stroke {
    fn convert(&self) -> StrokeOptions {
        let linecap = match self.linecap {
            usvg::LineCap::Butt => LineCap::Butt,
            usvg::LineCap::Square => LineCap::Square,
            usvg::LineCap::Round => LineCap::Round,
        };
        let linejoin = match self.linejoin {
            usvg::LineJoin::Miter => LineJoin::Miter,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
            usvg::LineJoin::Round => LineJoin::Round,
        };
        StrokeOptions::default()
            .with_line_width(self.width.get() as f32)
            .with_line_cap(linecap)
            .with_line_join(linejoin)
    }
}
impl<'l> Convert<'l, PathConvIter<'l>> for usvg::Path {
    fn convert(&'l self) -> PathConvIter<'l> {
        PathConvIter {
            iter: self.data.segments(),
            first: Point::new(0.0, 0.0),
            prev: Point::new(0.0, 0.0),
            deferred: None,
            needs_end: false,
        }
    }
}
impl Convert<'_, Color32> for (usvg::Color, f64) {
    fn convert(&self) -> Color32 {
        let (color, opacity) = *self;
        Color32::from_rgba_unmultiplied(color.red, color.green, color.blue, (opacity * 255.0) as u8)
    }
}
impl Convert<'_, Rect> for usvg::Rect {
    fn convert(&self) -> Rect {
        Rect::from_min_max(
            [self.left() as f32, self.top() as f32].into(),
            [self.right() as f32, self.bottom() as f32].into(),
        )
    }
}
