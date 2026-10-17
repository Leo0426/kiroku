//! 预览封面角标:`cargo run -p kiroku-core --example badge_demo -- <横版封面.jpg> <输出目录>`
//! 第一个参数为 `-` 时用渐变图代替。角标图读 `<设置目录>/watermarks`,没有则用内置图。
use kiroku_core::{
    badge::{user_dir, BadgeCfg, Corner, Corners, Phase},
    code::parse_code,
    organize::{process_image, Crop},
    settings::Settings,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let src = args.next().filter(|s| s != "-").map(|p| std::fs::read(p).expect("读取图片")).unwrap_or_else(|| {
        let img = image::RgbImage::from_fn(800, 538, |x, y| image::Rgb([(x / 4) as u8, (y / 3) as u8, 140]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    });
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let dir = user_dir(&Settings::default_path());
    let cfg = BadgeCfg { enabled: true, fanart: true, ..Default::default() };
    let spread = BadgeCfg {
        corners: Corners { cracked: Some(Corner::TopRight), leaked: Some(Corner::TopRight), definition: Some(Corner::BottomRight), ..Default::default() },
        ..cfg
    };
    for (name, file, cfg) in [("uc-4k", "SMPL-001-UC-4K.mp4", cfg), ("spread", "SMPL-001-UC-4K.mp4", spread), ("heyzo-leak", "HEYZO-1234-C-leak.mp4", cfg)] {
        let Some(stamp) = cfg.stamping(&Phase::of(&parse_code(file).unwrap()), Some(&dir)) else { continue };
        std::fs::write(out.join(format!("{name}-poster.jpg")), process_image(src.clone(), Some(Crop::PosterRight), Some(&stamp)).unwrap()).unwrap();
        std::fs::write(out.join(format!("{name}-fanart.jpg")), process_image(src.clone(), None, Some(&stamp)).unwrap()).unwrap();
    }
}
