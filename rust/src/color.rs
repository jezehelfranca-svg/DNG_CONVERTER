use image::RgbImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorProfile {
    Balanced,
    Vibrant,
    Portrait,
    Cinematic,
    None,
}

impl ColorProfile {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "vibrant" => ColorProfile::Vibrant,
            "portrait" => ColorProfile::Portrait,
            "cinematic" => ColorProfile::Cinematic,
            "none" | "flat" => ColorProfile::None,
            _ => ColorProfile::Balanced,
        }
    }
}

// Convert sRGB (0..255) to linear sRGB (0.0..1.0)
#[inline(always)]
fn srgb_to_linear(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

// Convert linear sRGB (0.0..1.0) back to sRGB (0..255)
#[inline(always)]
fn linear_to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

// Linear RGB to Oklab
#[inline(always)]
fn linear_rgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    let l_ok = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
    let a_ok = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
    let b_ok = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;

    (l_ok, a_ok, b_ok)
}

// Oklab to Linear RGB
#[inline(always)]
fn oklab_to_linear_rgb(l_ok: f32, a_ok: f32, b_ok: f32) -> (f32, f32, f32) {
    let l_ = l_ok + 0.3963377774 * a_ok + 0.2158037573 * b_ok;
    let m_ = l_ok - 0.1055613458 * a_ok - 0.0638541728 * b_ok;
    let s_ = l_ok - 0.0894841775 * a_ok - 1.2914855480 * b_ok;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    let r =  4.0767439362 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;

    (r, g, b)
}

// Normalized sigmoidal tone mapping: maps 0..1 to 0..1 with controllable contrast (k) and midpoint (x0)
#[inline(always)]
fn sigmoidal_curve(x: f32, k: f32, x0: f32) -> f32 {
    let s0 = 1.0 / (1.0 + (-k * (0.0 - x0)).exp());
    let s1 = 1.0 / (1.0 + (-k * (1.0 - x0)).exp());
    let sx = 1.0 / (1.0 + (-k * (x - x0)).exp());
    ((sx - s0) / (s1 - s0)).clamp(0.0, 1.0)
}

/// Applies perceptually uniform adaptive color optimization across the image.
pub fn apply_adaptive_grading(img: &mut RgbImage, profile: ColorProfile) {
    if profile == ColorProfile::None {
        return;
    }

    let (k, x0, vib_boost, warm_shift) = match profile {
        ColorProfile::Balanced => (2.4, 0.48, 0.14, 0.0),
        ColorProfile::Vibrant => (3.2, 0.46, 0.26, 0.0),
        ColorProfile::Portrait => (1.6, 0.50, 0.06, 0.0),
        ColorProfile::Cinematic => (3.6, 0.44, 0.10, 0.008),
        ColorProfile::None => return,
    };

    for pixel in img.pixels_mut() {
        let r_lin = srgb_to_linear(pixel[0]);
        let g_lin = srgb_to_linear(pixel[1]);
        let b_lin = srgb_to_linear(pixel[2]);

        let (l_ok, mut a_ok, mut b_ok) = linear_rgb_to_oklab(r_lin, g_lin, b_lin);

        // 1. Photographic Sigmoidal Lightness Tone Curve
        let l_graded = sigmoidal_curve(l_ok, k, x0);

        // 2. Smart Vibrance in Oklab: Chroma C = sqrt(a^2 + b^2)
        let chroma = (a_ok * a_ok + b_ok * b_ok).sqrt();
        // Boost muted colors more, keep already saturated tones intact
        let weight = (1.0 - (chroma / 0.25).clamp(0.0, 1.0)).max(0.1);
        let sat_factor = 1.0 + vib_boost * weight;

        a_ok *= sat_factor;
        b_ok = b_ok * sat_factor + warm_shift;

        let (r_out, g_out, b_out) = oklab_to_linear_rgb(l_graded, a_ok, b_ok);

        pixel[0] = linear_to_srgb(r_out);
        pixel[1] = linear_to_srgb(g_out);
        pixel[2] = linear_to_srgb(b_out);
    }
}