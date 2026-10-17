//! 生成应用图标源图(1024×1024 PNG):深色圆角方块 + 带缺口的琥珀色环 + 中心点。
//!     cargo run --release -p kiroku-core --example gen_icon -- <输出路径>
//! 然后在 apps/desktop 下运行 `pnpm tauri icon <输出路径>` 生成各平台尺寸。
//! 与 apps/desktop/public/favicon.svg、src/components/Logo.svelte 保持同一造型。

use image::{Rgba, RgbaImage};

const N: u32 = 1024;
const SS: u32 = 4; // 每像素 4×4 超采样

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

fn hex(h: u32) -> [f32; 3] {
    [((h >> 16) & 255) as f32, ((h >> 8) & 255) as f32, (h & 255) as f32]
}

/// 圆角矩形的有符号距离(中心在原点,半宽 hw,圆角 r)。
fn rrect(x: f32, y: f32, hw: f32, r: f32) -> f32 {
    let qx = x.abs() - (hw - r);
    let qy = y.abs() - (hw - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "icon-source.png".into());
    let (bg_top, bg_bot) = (hex(0x2a2630), hex(0x0e0d11));
    let (am_a, am_b) = (hex(0xf6c983), hex(0xdf9233));
    let c = N as f32 / 2.0;
    // macOS 图标网格:主体 824,圆角约 22%
    let (hw, rad) = (412.0, 186.0);
    let (ring_r, ring_w, dot_r) = (238.0, 92.0, 52.0);
    // 缺口:以右上 45° 为中心,张角 34°
    let (gap_c, gap_half) = ((-45.0f32).to_radians(), 17.0f32.to_radians());

    let mut img = RgbaImage::new(N, N);
    for py in 0..N {
        for px in 0..N {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32 - c;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32 - c;
                    if rrect(x, y, hw, rad) > 0.0 {
                        continue; // 透明
                    }
                    // 背景:竖向渐变 + 顶部一层很淡的高光
                    let t = (y + hw) / (2.0 * hw);
                    let mut col = mix(bg_top, bg_bot, t);
                    let glow = (1.0 - ((x * x + (y + 300.0) * (y + 300.0)).sqrt() / 520.0)).max(0.0) * 14.0;
                    col = [col[0] + glow, col[1] + glow, col[2] + glow];

                    let d = (x * x + y * y).sqrt();
                    let ang = y.atan2(x);
                    let mut da = (ang - gap_c).abs();
                    if da > std::f32::consts::PI {
                        da = 2.0 * std::f32::consts::PI - da;
                    }
                    let amber = mix(am_a, am_b, ((x + y) / (2.0 * ring_r) + 1.0) / 2.0);
                    let in_ring = (d - ring_r).abs() <= ring_w / 2.0 && da > gap_half;
                    if in_ring || d <= dot_r {
                        col = amber;
                    }
                    acc[0] += col[0];
                    acc[1] += col[1];
                    acc[2] += col[2];
                    acc[3] += 1.0;
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let rgb = if acc[3] > 0.0 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3]] } else { [0.0; 3] };
            img.put_pixel(px, py, Rgba([rgb[0].min(255.0) as u8, rgb[1].min(255.0) as u8, rgb[2].min(255.0) as u8, (a * 255.0).round() as u8]));
        }
    }
    img.save(&out).expect("write png");
    println!("wrote {out}");
}
