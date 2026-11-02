//! A thin layer over GDI+'s flat API (gdiplus.dll, part of every Windows): antialiased
//! paths, gradients, soft shadows and text, enough to draw the status window's desk
//! (RULES.md B4). Colours are 0xAARRGGBB; coordinates are logical pixels, scaled once
//! by the caller's world transform.

use crate::win32::wide;
use std::ffi::c_void;
use std::ptr;

type Gp = *mut c_void;

#[repr(C)]
struct StartupInput {
    version: u32,
    callback: *mut c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointF {
    x: f32,
    y: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

// Loaded on first use rather than linked: the background client shares this exe and
// never draws, so it must not pay for gdiplus.dll (RULES.md R1).
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

#[allow(non_snake_case)]
struct Api {
    GdiplusStartup: unsafe extern "system" fn(*mut usize, *const StartupInput, *mut c_void) -> i32,
    GdipCreateFromHDC: unsafe extern "system" fn(*mut c_void, *mut Gp) -> i32,
    GdipDeleteGraphics: unsafe extern "system" fn(Gp) -> i32,
    GdipSetSmoothingMode: unsafe extern "system" fn(Gp, i32) -> i32,
    GdipSetPixelOffsetMode: unsafe extern "system" fn(Gp, i32) -> i32,
    GdipSetTextRenderingHint: unsafe extern "system" fn(Gp, i32) -> i32,
    GdipScaleWorldTransform: unsafe extern "system" fn(Gp, f32, f32, i32) -> i32,
    GdipTranslateWorldTransform: unsafe extern "system" fn(Gp, f32, f32, i32) -> i32,
    GdipSaveGraphics: unsafe extern "system" fn(Gp, *mut u32) -> i32,
    GdipRestoreGraphics: unsafe extern "system" fn(Gp, u32) -> i32,
    GdipSetClipPath: unsafe extern "system" fn(Gp, Gp, i32) -> i32,
    GdipResetClip: unsafe extern "system" fn(Gp) -> i32,
    GdipCreatePath: unsafe extern "system" fn(i32, *mut Gp) -> i32,
    GdipDeletePath: unsafe extern "system" fn(Gp) -> i32,
    GdipAddPathLine: unsafe extern "system" fn(Gp, f32, f32, f32, f32) -> i32,
    GdipAddPathBezier: unsafe extern "system" fn(Gp, f32, f32, f32, f32, f32, f32, f32, f32) -> i32,
    GdipAddPathArc: unsafe extern "system" fn(Gp, f32, f32, f32, f32, f32, f32) -> i32,
    GdipAddPathEllipse: unsafe extern "system" fn(Gp, f32, f32, f32, f32) -> i32,
    GdipStartPathFigure: unsafe extern "system" fn(Gp) -> i32,
    GdipClosePathFigure: unsafe extern "system" fn(Gp) -> i32,
    GdipCreateSolidFill: unsafe extern "system" fn(u32, *mut Gp) -> i32,
    GdipCreateLineBrush: unsafe extern "system" fn(*const PointF, *const PointF, u32, u32, i32, *mut Gp) -> i32,
    GdipCreatePathGradientFromPath: unsafe extern "system" fn(Gp, *mut Gp) -> i32,
    GdipSetPathGradientCenterColor: unsafe extern "system" fn(Gp, u32) -> i32,
    GdipSetPathGradientSurroundColorsWithCount: unsafe extern "system" fn(Gp, *const u32, *mut i32) -> i32,
    GdipDeleteBrush: unsafe extern "system" fn(Gp) -> i32,
    GdipFillPath: unsafe extern "system" fn(Gp, Gp, Gp) -> i32,
    GdipCreatePen1: unsafe extern "system" fn(u32, f32, i32, *mut Gp) -> i32,
    GdipSetPenLineCap197819: unsafe extern "system" fn(Gp, i32, i32, i32) -> i32,
    GdipSetPenLineJoin: unsafe extern "system" fn(Gp, i32) -> i32,
    GdipSetPenDashArray: unsafe extern "system" fn(Gp, *const f32, i32) -> i32,
    GdipSetPenDashOffset: unsafe extern "system" fn(Gp, f32) -> i32,
    GdipDeletePen: unsafe extern "system" fn(Gp) -> i32,
    GdipDrawPath: unsafe extern "system" fn(Gp, Gp, Gp) -> i32,
    GdipCreateFontFamilyFromName: unsafe extern "system" fn(*const u16, Gp, *mut Gp) -> i32,
    GdipDeleteFontFamily: unsafe extern "system" fn(Gp) -> i32,
    GdipCreateFont: unsafe extern "system" fn(Gp, f32, i32, i32, *mut Gp) -> i32,
    GdipDeleteFont: unsafe extern "system" fn(Gp) -> i32,
    GdipCreateStringFormat: unsafe extern "system" fn(i32, u16, *mut Gp) -> i32,
    GdipSetStringFormatAlign: unsafe extern "system" fn(Gp, i32) -> i32,
    GdipSetStringFormatLineAlign: unsafe extern "system" fn(Gp, i32) -> i32,
    GdipDeleteStringFormat: unsafe extern "system" fn(Gp) -> i32,
    GdipDrawString: unsafe extern "system" fn(Gp, *const u16, i32, Gp, *const RectF, Gp, Gp) -> i32,
}

static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();

/// gdiplus.dll's functions, or None if it cannot be loaded.
fn load() -> Option<Api> {
    // from System32 only (LOAD_LIBRARY_SEARCH_SYSTEM32), never a look-alike next to the exe
    let dll = unsafe { LoadLibraryExW(wide("gdiplus.dll").as_ptr(), ptr::null_mut(), 0x800) };
    if dll.is_null() {
        return None;
    }
    let get = |name: &str| {
        let f = unsafe { GetProcAddress(dll, format!("{name}\0").as_ptr()) };
        (!f.is_null()).then_some(f)
    };
    // SAFETY: each pointer is the export of that name, whose signature the field declares
    unsafe {
        Some(Api {
            GdiplusStartup: std::mem::transmute::<*mut c_void, _>(get("GdiplusStartup")?),
            GdipCreateFromHDC: std::mem::transmute::<*mut c_void, _>(get("GdipCreateFromHDC")?),
            GdipDeleteGraphics: std::mem::transmute::<*mut c_void, _>(get("GdipDeleteGraphics")?),
            GdipSetSmoothingMode: std::mem::transmute::<*mut c_void, _>(get("GdipSetSmoothingMode")?),
            GdipSetPixelOffsetMode: std::mem::transmute::<*mut c_void, _>(get("GdipSetPixelOffsetMode")?),
            GdipSetTextRenderingHint: std::mem::transmute::<*mut c_void, _>(get("GdipSetTextRenderingHint")?),
            GdipScaleWorldTransform: std::mem::transmute::<*mut c_void, _>(get("GdipScaleWorldTransform")?),
            GdipTranslateWorldTransform: std::mem::transmute::<*mut c_void, _>(get("GdipTranslateWorldTransform")?),
            GdipSaveGraphics: std::mem::transmute::<*mut c_void, _>(get("GdipSaveGraphics")?),
            GdipRestoreGraphics: std::mem::transmute::<*mut c_void, _>(get("GdipRestoreGraphics")?),
            GdipSetClipPath: std::mem::transmute::<*mut c_void, _>(get("GdipSetClipPath")?),
            GdipResetClip: std::mem::transmute::<*mut c_void, _>(get("GdipResetClip")?),
            GdipCreatePath: std::mem::transmute::<*mut c_void, _>(get("GdipCreatePath")?),
            GdipDeletePath: std::mem::transmute::<*mut c_void, _>(get("GdipDeletePath")?),
            GdipAddPathLine: std::mem::transmute::<*mut c_void, _>(get("GdipAddPathLine")?),
            GdipAddPathBezier: std::mem::transmute::<*mut c_void, _>(get("GdipAddPathBezier")?),
            GdipAddPathArc: std::mem::transmute::<*mut c_void, _>(get("GdipAddPathArc")?),
            GdipAddPathEllipse: std::mem::transmute::<*mut c_void, _>(get("GdipAddPathEllipse")?),
            GdipStartPathFigure: std::mem::transmute::<*mut c_void, _>(get("GdipStartPathFigure")?),
            GdipClosePathFigure: std::mem::transmute::<*mut c_void, _>(get("GdipClosePathFigure")?),
            GdipCreateSolidFill: std::mem::transmute::<*mut c_void, _>(get("GdipCreateSolidFill")?),
            GdipCreateLineBrush: std::mem::transmute::<*mut c_void, _>(get("GdipCreateLineBrush")?),
            GdipCreatePathGradientFromPath: std::mem::transmute::<*mut c_void, _>(get("GdipCreatePathGradientFromPath")?),
            GdipSetPathGradientCenterColor: std::mem::transmute::<*mut c_void, _>(get("GdipSetPathGradientCenterColor")?),
            GdipSetPathGradientSurroundColorsWithCount: std::mem::transmute::<*mut c_void, _>(get("GdipSetPathGradientSurroundColorsWithCount")?),
            GdipDeleteBrush: std::mem::transmute::<*mut c_void, _>(get("GdipDeleteBrush")?),
            GdipFillPath: std::mem::transmute::<*mut c_void, _>(get("GdipFillPath")?),
            GdipCreatePen1: std::mem::transmute::<*mut c_void, _>(get("GdipCreatePen1")?),
            GdipSetPenLineCap197819: std::mem::transmute::<*mut c_void, _>(get("GdipSetPenLineCap197819")?),
            GdipSetPenLineJoin: std::mem::transmute::<*mut c_void, _>(get("GdipSetPenLineJoin")?),
            GdipSetPenDashArray: std::mem::transmute::<*mut c_void, _>(get("GdipSetPenDashArray")?),
            GdipSetPenDashOffset: std::mem::transmute::<*mut c_void, _>(get("GdipSetPenDashOffset")?),
            GdipDeletePen: std::mem::transmute::<*mut c_void, _>(get("GdipDeletePen")?),
            GdipDrawPath: std::mem::transmute::<*mut c_void, _>(get("GdipDrawPath")?),
            GdipCreateFontFamilyFromName: std::mem::transmute::<*mut c_void, _>(get("GdipCreateFontFamilyFromName")?),
            GdipDeleteFontFamily: std::mem::transmute::<*mut c_void, _>(get("GdipDeleteFontFamily")?),
            GdipCreateFont: std::mem::transmute::<*mut c_void, _>(get("GdipCreateFont")?),
            GdipDeleteFont: std::mem::transmute::<*mut c_void, _>(get("GdipDeleteFont")?),
            GdipCreateStringFormat: std::mem::transmute::<*mut c_void, _>(get("GdipCreateStringFormat")?),
            GdipSetStringFormatAlign: std::mem::transmute::<*mut c_void, _>(get("GdipSetStringFormatAlign")?),
            GdipSetStringFormatLineAlign: std::mem::transmute::<*mut c_void, _>(get("GdipSetStringFormatLineAlign")?),
            GdipDeleteStringFormat: std::mem::transmute::<*mut c_void, _>(get("GdipDeleteStringFormat")?),
            GdipDrawString: std::mem::transmute::<*mut c_void, _>(get("GdipDrawString")?),
        })
    }
}

fn api() -> &'static Api {
    API.get().and_then(Option::as_ref).expect("GDI+ used before startup()")
}

const SMOOTHING_ANTIALIAS: i32 = 4;
const PIXEL_OFFSET_HALF: i32 = 4;
const TEXT_ANTIALIAS_GRIDFIT: i32 = 3;
const UNIT_WORLD: i32 = 0;
const UNIT_PIXEL: i32 = 2;
const CAP_ROUND: i32 = 2;
const DASH_CAP_ROUND: i32 = 2;
const JOIN_ROUND: i32 = 2;
const WRAP_TILE_FLIP_XY: i32 = 3;
const FILL_WINDING: i32 = 1;
const COMBINE_INTERSECT: i32 = 1;
const NO_WRAP: i32 = 0x1000;

/// Starts GDI+ once for the process; it stays up until the process ends.
pub fn startup() -> bool {
    if API.get_or_init(load).is_none() {
        return false;
    }
    let input = StartupInput { version: 1, callback: ptr::null_mut(), suppress_background_thread: 0, suppress_external_codecs: 0 };
    let mut token = 0usize;
    unsafe { (api().GdiplusStartup)(&mut token, &input, ptr::null_mut()) == 0 }
}

/// Opaque colour from 0xRRGGBB.
pub const fn rgb(c: u32) -> u32 {
    0xFF00_0000 | c
}

/// Colour from 0xRRGGBB and an opacity 0..=1.
pub fn rgba(c: u32, a: f32) -> u32 {
    ((a.clamp(0.0, 1.0) * 255.0).round() as u32) << 24 | (c & 0xFF_FFFF)
}

/// A path built with pen-plotter calls; the figure stays open until `close`.
pub struct Path {
    raw: Gp,
    at: (f32, f32),
}

impl Path {
    pub fn new() -> Path {
        let mut raw = ptr::null_mut();
        unsafe { (api().GdipCreatePath)(FILL_WINDING, &mut raw) };
        Path { raw, at: (0.0, 0.0) }
    }

    pub fn move_to(mut self, x: f32, y: f32) -> Path {
        unsafe { (api().GdipStartPathFigure)(self.raw) };
        self.at = (x, y);
        self
    }

    pub fn line_to(mut self, x: f32, y: f32) -> Path {
        unsafe { (api().GdipAddPathLine)(self.raw, self.at.0, self.at.1, x, y) };
        self.at = (x, y);
        self
    }

    pub fn curve_to(mut self, c1: (f32, f32), c2: (f32, f32), x: f32, y: f32) -> Path {
        unsafe { (api().GdipAddPathBezier)(self.raw, self.at.0, self.at.1, c1.0, c1.1, c2.0, c2.1, x, y) };
        self.at = (x, y);
        self
    }

    pub fn quad_to(self, c: (f32, f32), x: f32, y: f32) -> Path {
        let (x0, y0) = self.at;
        let c1 = (x0 + 2.0 / 3.0 * (c.0 - x0), y0 + 2.0 / 3.0 * (c.1 - y0));
        let c2 = (x + 2.0 / 3.0 * (c.0 - x), y + 2.0 / 3.0 * (c.1 - y));
        self.curve_to(c1, c2, x, y)
    }

    pub fn close(self) -> Path {
        unsafe { (api().GdipClosePathFigure)(self.raw) };
        self
    }

    pub fn poly(points: &[(f32, f32)]) -> Path {
        let mut p = Path::new().move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] {
            p = p.line_to(x, y);
        }
        p.close()
    }

    pub fn rounded(self, x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
        let d = (r * 2.0).min(w).min(h);
        unsafe {
            (api().GdipStartPathFigure)(self.raw);
            (api().GdipAddPathArc)(self.raw, x, y, d, d, 180.0, 90.0);
            (api().GdipAddPathArc)(self.raw, x + w - d, y, d, d, 270.0, 90.0);
            (api().GdipAddPathArc)(self.raw, x + w - d, y + h - d, d, d, 0.0, 90.0);
            (api().GdipAddPathArc)(self.raw, x, y + h - d, d, d, 90.0, 90.0);
            (api().GdipClosePathFigure)(self.raw);
        }
        self
    }

    pub fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
        Path::new().rounded(x, y, w, h, r)
    }

    pub fn ellipse(x: f32, y: f32, w: f32, h: f32) -> Path {
        let p = Path::new();
        unsafe { (api().GdipAddPathEllipse)(p.raw, x, y, w, h) };
        p
    }
}

impl Drop for Path {
    fn drop(&mut self) {
        unsafe { (api().GdipDeletePath)(self.raw) };
    }
}

/// A text face, created once and kept.
pub struct Font {
    raw: Gp,
    family: Gp,
}

impl Font {
    pub fn new(face: &str, size: f32, bold: bool) -> Font {
        let mut family = ptr::null_mut();
        let mut raw = ptr::null_mut();
        unsafe {
            if (api().GdipCreateFontFamilyFromName)(wide(face).as_ptr(), ptr::null_mut(), &mut family) != 0 {
                (api().GdipCreateFontFamilyFromName)(wide("Microsoft YaHei UI").as_ptr(), ptr::null_mut(), &mut family);
            }
            (api().GdipCreateFont)(family, size, if bold { 1 } else { 0 }, UNIT_PIXEL, &mut raw);
        }
        Font { raw, family }
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            (api().GdipDeleteFont)(self.raw);
            (api().GdipDeleteFontFamily)(self.family);
        }
    }
}

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A drawing surface on a device context, for the length of one paint.
pub struct Canvas {
    g: Gp,
}

impl Canvas {
    pub fn on(hdc: *mut c_void, scale: f32) -> Canvas {
        let mut g = ptr::null_mut();
        unsafe {
            (api().GdipCreateFromHDC)(hdc, &mut g);
            (api().GdipSetSmoothingMode)(g, SMOOTHING_ANTIALIAS);
            (api().GdipSetPixelOffsetMode)(g, PIXEL_OFFSET_HALF);
            (api().GdipSetTextRenderingHint)(g, TEXT_ANTIALIAS_GRIDFIT);
            (api().GdipScaleWorldTransform)(g, scale, scale, 0);
        }
        Canvas { g }
    }

    /// Runs `f` with the origin moved and the scale changed, then puts both back.
    pub fn within(&self, dx: f32, dy: f32, s: f32, f: impl FnOnce(&Canvas)) {
        let mut state = 0;
        unsafe {
            (api().GdipSaveGraphics)(self.g, &mut state);
            (api().GdipTranslateWorldTransform)(self.g, dx, dy, 0);
            (api().GdipScaleWorldTransform)(self.g, s, s, 0);
        }
        f(self);
        unsafe { (api().GdipRestoreGraphics)(self.g, state) };
    }

    /// Runs `f` with drawing limited to inside `path`.
    pub fn clipped(&self, path: &Path, f: impl FnOnce(&Canvas)) {
        let mut state = 0;
        unsafe {
            (api().GdipSaveGraphics)(self.g, &mut state);
            (api().GdipSetClipPath)(self.g, path.raw, COMBINE_INTERSECT);
        }
        f(self);
        unsafe {
            (api().GdipResetClip)(self.g);
            (api().GdipRestoreGraphics)(self.g, state);
        }
    }

    pub fn fill(&self, path: &Path, color: u32) {
        let mut b = ptr::null_mut();
        unsafe {
            (api().GdipCreateSolidFill)(color, &mut b);
            (api().GdipFillPath)(self.g, b, path.raw);
            (api().GdipDeleteBrush)(b);
        }
    }

    /// Fills with a straight gradient from `from` to `to`.
    pub fn fill_linear(&self, path: &Path, from: (f32, f32), to: (f32, f32), c1: u32, c2: u32) {
        let (p1, p2) = (PointF { x: from.0, y: from.1 }, PointF { x: to.0, y: to.1 });
        let mut b = ptr::null_mut();
        unsafe {
            (api().GdipCreateLineBrush)(&p1, &p2, c1, c2, WRAP_TILE_FLIP_XY, &mut b);
            (api().GdipFillPath)(self.g, b, path.raw);
            (api().GdipDeleteBrush)(b);
        }
    }

    /// A soft blot: `color` in the middle of the ellipse, fading to nothing at its rim.
    pub fn soft(&self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let path = Path::ellipse(x, y, w, h);
        let mut b = ptr::null_mut();
        let clear = [color & 0x00FF_FFFF];
        let mut n = 1;
        unsafe {
            (api().GdipCreatePathGradientFromPath)(path.raw, &mut b);
            (api().GdipSetPathGradientCenterColor)(b, color);
            (api().GdipSetPathGradientSurroundColorsWithCount)(b, clear.as_ptr(), &mut n);
            (api().GdipFillPath)(self.g, b, path.raw);
            (api().GdipDeleteBrush)(b);
        }
    }

    pub fn stroke(&self, path: &Path, color: u32, width: f32) {
        self.stroke_dashed(path, color, width, &[], 0.0);
    }

    /// Strokes with round caps and joins; `dash` and `offset` are in multiples of the width.
    pub fn stroke_dashed(&self, path: &Path, color: u32, width: f32, dash: &[f32], offset: f32) {
        let mut pen = ptr::null_mut();
        unsafe {
            (api().GdipCreatePen1)(color, width, UNIT_WORLD, &mut pen);
            (api().GdipSetPenLineCap197819)(pen, CAP_ROUND, CAP_ROUND, DASH_CAP_ROUND);
            (api().GdipSetPenLineJoin)(pen, JOIN_ROUND);
            if !dash.is_empty() {
                (api().GdipSetPenDashArray)(pen, dash.as_ptr(), dash.len() as i32);
                (api().GdipSetPenDashOffset)(pen, offset);
            }
            (api().GdipDrawPath)(self.g, pen, path.raw);
            (api().GdipDeletePen)(pen);
        }
    }

    /// One line of text in the box (x, y, w, h), vertically centred.
    pub fn text(&self, s: &str, font: &Font, color: u32, x: f32, y: f32, w: f32, h: f32, align: Align) {
        let text = s.encode_utf16().collect::<Vec<u16>>();
        let rect = RectF { x, y, w, h };
        let mut format = ptr::null_mut();
        let mut b = ptr::null_mut();
        unsafe {
            (api().GdipCreateStringFormat)(NO_WRAP, 0, &mut format);
            (api().GdipSetStringFormatAlign)(format, align as i32);
            (api().GdipSetStringFormatLineAlign)(format, 1);
            (api().GdipCreateSolidFill)(color, &mut b);
            (api().GdipDrawString)(self.g, text.as_ptr(), text.len() as i32, font.raw, &rect, format, b);
            (api().GdipDeleteBrush)(b);
            (api().GdipDeleteStringFormat)(format);
        }
    }

    /// Text that may wrap onto further lines inside (x, y, w, h), from the top.
    pub fn paragraph(&self, s: &str, font: &Font, color: u32, x: f32, y: f32, w: f32, h: f32) {
        let text = s.encode_utf16().collect::<Vec<u16>>();
        let rect = RectF { x, y, w, h };
        let mut format = ptr::null_mut();
        let mut b = ptr::null_mut();
        unsafe {
            (api().GdipCreateStringFormat)(0, 0, &mut format);
            (api().GdipCreateSolidFill)(color, &mut b);
            (api().GdipDrawString)(self.g, text.as_ptr(), text.len() as i32, font.raw, &rect, format, b);
            (api().GdipDeleteBrush)(b);
            (api().GdipDeleteStringFormat)(format);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe { (api().GdipDeleteGraphics)(self.g) };
    }
}
