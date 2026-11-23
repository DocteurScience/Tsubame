//! Préparation des images jointes à un post, avant l'envoi.
//!
//! Bluesky refuse les images de plus d'un million d'octets. On redresse
//! l'image selon son orientation d'appareil photo, on la réduit à
//! `MAX_SIDE` pixels de côté, puis on la réencode (JPEG, ou PNG si elle a de
//! la transparence) en baissant la qualité jusqu'à passer sous la limite.
//! Le réencodage supprime au passage les métadonnées, dont la position GPS
//! des photos.

use gtk::gdk_pixbuf::{InterpType, Pixbuf, PixbufLoader};
use gtk::prelude::*;

/// Taille maximale acceptée par Bluesky pour une image.
pub const MAX_BYTES: usize = 1_000_000;
/// Plus grand côté conservé.
const MAX_SIDE: i32 = 2000;
const JPEG_QUALITIES: [&str; 5] = ["90", "82", "74", "66", "58"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

fn decode(bytes: &[u8]) -> Result<Pixbuf, String> {
    let loader = PixbufLoader::new();
    loader.write(bytes).map_err(|e| e.to_string())?;
    loader.close().map_err(|e| e.to_string())?;
    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| "image illisible".to_owned())?;
    Ok(pixbuf.apply_embedded_orientation().unwrap_or(pixbuf))
}

fn scale_to(pixbuf: &Pixbuf, max_side: i32) -> Result<Pixbuf, String> {
    let (width, height) = (pixbuf.width(), pixbuf.height());
    let longest = width.max(height);
    if longest <= max_side {
        return Ok(pixbuf.clone());
    }
    let ratio = f64::from(max_side) / f64::from(longest);
    let w = ((f64::from(width) * ratio).round() as i32).max(1);
    let h = ((f64::from(height) * ratio).round() as i32).max(1);
    pixbuf
        .scale_simple(w, h, InterpType::Bilinear)
        .ok_or_else(|| "mémoire insuffisante".to_owned())
}

/// Image prête à l'envoi, ou la raison du refus. Lent sur les grandes
/// photos : à appeler hors de la boucle GTK.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, String> {
    let original = decode(bytes)?;
    let mut side = MAX_SIDE;
    while side >= 200 {
        let pixbuf = scale_to(&original, side)?;
        let candidates: Vec<(&'static str, Vec<(&str, &str)>)> = if pixbuf.has_alpha() {
            vec![("png", vec![("compression", "9")])]
        } else {
            JPEG_QUALITIES
                .iter()
                .map(|q| ("jpeg", vec![("quality", *q)]))
                .collect()
        };
        for (format, options) in candidates {
            let encoded = pixbuf
                .save_to_bufferv(format, &options)
                .map_err(|e| e.to_string())?;
            if encoded.len() <= MAX_BYTES {
                return Ok(Prepared {
                    bytes: encoded,
                    mime: if format == "png" {
                        "image/png"
                    } else {
                        "image/jpeg"
                    },
                    width: pixbuf.width() as u32,
                    height: pixbuf.height() as u32,
                });
            }
        }
        // Toujours trop lourde : réduire encore.
        side = side * 3 / 4;
    }
    Err("image trop lourde".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::gdk_pixbuf::Colorspace;

    /// Image de bruit, difficile à compresser, encodée en PNG.
    fn noisy_png(width: i32, height: i32, alpha: bool) -> Vec<u8> {
        let pixbuf = Pixbuf::new(Colorspace::Rgb, alpha, 8, width, height).unwrap();
        let mut seed: u32 = 12345;
        let channels = if alpha { 4 } else { 3 };
        for y in 0..height {
            for x in 0..width {
                let mut pixel = [0u8; 4];
                for value in pixel.iter_mut().take(channels) {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    *value = (seed >> 16) as u8;
                }
                pixbuf.put_pixel(
                    x as u32,
                    y as u32,
                    pixel[0],
                    pixel[1],
                    pixel[2],
                    if alpha { pixel[3] } else { 255 },
                );
            }
        }
        pixbuf.save_to_bufferv("png", &[]).unwrap()
    }

    #[test]
    fn large_photos_are_reduced_under_the_limit() {
        let original = noisy_png(2600, 1300, false);
        assert!(original.len() > MAX_BYTES);
        let prepared = prepare(&original).unwrap();
        assert_eq!(prepared.mime, "image/jpeg");
        assert!(prepared.bytes.len() <= MAX_BYTES);
        assert!(prepared.width <= 2000 && prepared.height <= 2000);
        // Proportions conservées (2:1), au pixel près.
        assert!((prepared.width as i64 - 2 * prepared.height as i64).abs() <= 2);
    }

    #[test]
    fn transparency_is_kept() {
        let prepared = prepare(&noisy_png(300, 200, true)).unwrap();
        assert_eq!(prepared.mime, "image/png");
        assert_eq!((prepared.width, prepared.height), (300, 200));
    }

    #[test]
    fn what_is_not_an_image_is_refused() {
        assert!(prepare(b"pas une image").is_err());
    }
}
