//! Read-only avatar loading through the configured image transport.
use crate::net::Http;
use std::io::{self, Cursor};

pub(crate) fn validate_url(url: &str) -> io::Result<()> {
    match reqwest::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.has_host() => Ok(()),
        _ => Err(io::Error::other("演员头像网址必须是有效的 HTTP 或 HTTPS 地址")),
    }
}

/// Return a small, decoded PNG. Never pass an upstream HTML error page to the UI.
pub async fn fetch(http: &Http, url: &str) -> io::Result<Vec<u8>> {
    validate_url(url)?;
    let (status, bytes) = http.get_image_bytes(url, 8 * 1024 * 1024).await
        .map_err(|_| io::Error::other("头像下载失败"))?;
    if !(200..300).contains(&status) {
        return Err(io::Error::other(format!("头像下载失败（HTTP {status}）")));
    }
    tokio::task::spawn_blocking(move || thumbnail(&bytes)).await.map_err(io::Error::other)?
}

pub(crate) fn thumbnail(bytes: &[u8]) -> io::Result<Vec<u8>> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| io::Error::other("头像不是有效图片"))?;
    let orientation = decoder.orientation().map_err(io::Error::other)?;
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    img.apply_orientation(orientation);
    let mut out = Cursor::new(Vec::new());
    img.thumbnail(256, 256).write_to(&mut out, image::ImageFormat::Png).map_err(io::Error::other)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{Read, Write}, time::Duration};

    #[tokio::test]
    async fn fetches_hotlink_protected_avatar_with_referer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(600, 300).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = [0; 4096];
            let len = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
            let allowed = request.contains("referer: http://127.0.0.1/");
            let status = if allowed {200} else {403};
            let body = if allowed {png.into_inner()} else {b"Forbidden".to_vec()};
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        });
        let http = Http::new(Duration::ZERO, None).unwrap().for_images();
        let bytes = fetch(&http, &format!("http://{address}/avatar.jpg")).await.unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (256, 128));
        server.join().unwrap();
    }

    #[test]
    fn rejects_html_and_excessive_dimensions() {
        assert!(thumbnail(b"<html>Access denied</html>").is_err());
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4097, 1).write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(thumbnail(&png.into_inner()).is_err());
    }
}
