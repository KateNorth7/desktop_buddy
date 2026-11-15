//! On-device text recognition of one captured window image (Windows.Media.Ocr), in the
//! user's profile languages. Nothing leaves the device here.
use base64::Engine;
use windows::{
    Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

/// Balanced WinRT initialization on the calling worker thread.
struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { RoUninitialize() };
        }
    }
}
fn failed(_: windows::core::Error) -> String {
    "Text recognition failed".into()
}

pub(crate) fn recognize_text(base64_jpeg: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_jpeg)
        .map_err(|_| "Invalid image")?;
    if bytes.is_empty() {
        return Err("Invalid image".into());
    }
    let _apartment = Apartment(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok());
    let stream = InMemoryRandomAccessStream::new().map_err(failed)?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(failed)?;
    writer.WriteBytes(&bytes).map_err(failed)?;
    writer.StoreAsync().map_err(failed)?.get().map_err(failed)?;
    writer.DetachStream().map_err(failed)?;
    stream.Seek(0).map_err(failed)?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|_| "Invalid image")?
        .get()
        .map_err(|_| "Invalid image")?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(failed)?
        .get()
        .map_err(failed)?;
    // OCR accepts Gray8 or Bgra8 bitmaps.
    let bitmap = SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8).map_err(failed)?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| "No text recognition language is installed")?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(failed)?
        .get()
        .map_err(failed)?;
    let mut lines = Vec::new();
    for line in result.Lines().map_err(failed)? {
        let text = line.Text().map_err(failed)?.to_string();
        if !text.trim().is_empty() {
            lines.push(text);
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_text_is_recognized_on_device_in_reading_order() {
        let image = base64::engine::general_purpose::STANDARD.encode(include_bytes!("fixture.jpg"));
        let text = recognize_text(&image).unwrap().to_lowercase();
        let title = text.find("interview").expect("title recognized");
        let status = text.find("not been sent").expect("status line recognized");
        assert!(title < status, "top line comes first");
        assert!(recognize_text("bm90IGFuIGltYWdl").is_err());
    }
}
