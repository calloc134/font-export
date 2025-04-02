use clap::Parser; // clap を使うために追加
use std::ffi::{OsStr, c_void};
use std::fs;
use std::io::Write;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf; // PathBuf を使うために追加

use thiserror::Error;

// Windows API 関連
use windows::{
    Win32::Graphics::Gdi::{
        CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        DEFAULT_QUALITY, DeleteDC, DeleteObject, FF_DONTCARE, FW_NORMAL, GDI_ERROR, GetFontData,
        HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject,
    },
    core::{Error as WinError, PCWSTR},
};

/// --- コマンドライン引数定義 (clap を使用) ---
#[derive(Parser, Debug)]
#[command(version, about = "Extracts font data from an installed font.", long_about = None)]
struct Args {
    /// Name of the font to extract (e.g., "Arial", "Times New Roman")
    #[arg(long, short)]
    font_name: String,

    /// Directory where the font file should be saved
    #[arg(long, short, default_value = ".")]
    output_dir: PathBuf, // 保存先ディレクトリを PathBuf で受け取る
}

/// --- カスタムエラー型定義 ---
#[derive(Error, Debug)]
pub enum FontExtractorError {
    #[error("Windows API call '{api_name}' failed: {source}")]
    WinApi { api_name: String, source: WinError },
    #[error("Font '{font_name}' reported size 0 or could not be read.")]
    ZeroSizeFont { font_name: String },
    #[error("GetFontData reported unexpected size: expected {expected}, got {got}")]
    FontDataSizeMismatch { expected: u32, got: u32 },
    #[error("Failed to create/ensure output directory or file '{path}': {source}")]
    FileCreate {
        path: String,
        source: std::io::Error,
    },
    #[error("Failed to write to output file '{path}': {source}")]
    FileWrite {
        path: String,
        source: std::io::Error,
    },
}

/// --- RAII ラッパー: SafeDC ---
struct SafeDC(HDC);
impl SafeDC {
    fn new() -> Result<Self, FontExtractorError> {
        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.is_invalid() {
            Err(FontExtractorError::WinApi {
                api_name: "CreateCompatibleDC".to_string(),
                source: WinError::from_win32(),
            })
        } else {
            Ok(Self(hdc))
        }
    }
    fn get(&self) -> HDC {
        self.0
    }
}
impl Drop for SafeDC {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { DeleteDC(self.0) };
        }
    }
}

/// --- RAII ラッパー: SafeFont ---
struct SafeFont(HFONT);
impl SafeFont {
    fn create(font_name: &str) -> Result<Self, FontExtractorError> {
        let font_name_wide: Vec<u16> = OsStr::new(font_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let pcwstr_font_name = PCWSTR(font_name_wide.as_ptr());
        let font = unsafe {
            CreateFontW(
                0,
                0,
                0,
                0,
                FW_NORMAL.0.try_into().unwrap(),
                0,
                0,
                0,
                DEFAULT_CHARSET.0.into(),
                OUT_DEFAULT_PRECIS.0.into(),
                CLIP_DEFAULT_PRECIS.0.into(),
                DEFAULT_QUALITY.0.into(),
                (DEFAULT_PITCH.0 | FF_DONTCARE.0).into(),
                pcwstr_font_name,
            )
        };
        if font.is_invalid() {
            Err(FontExtractorError::WinApi {
                api_name: format!("CreateFontW (font: '{}')", font_name),
                source: WinError::from_win32(),
            })
        } else {
            Ok(Self(font))
        }
    }
    fn get(&self) -> HFONT {
        self.0
    }
}
impl Drop for SafeFont {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { DeleteObject(self.0) };
        }
    }
}

/// --- RAII ラッパー: FontSelector ---
struct FontSelector<'dc> {
    dc: &'dc SafeDC,
    old_font: Option<HGDIOBJ>,
    _marker: PhantomData<&'dc ()>,
}
impl<'dc> FontSelector<'dc> {
    fn select(dc: &'dc SafeDC, font: &SafeFont) -> Result<Self, FontExtractorError> {
        let old_font = unsafe { SelectObject(dc.get(), font.get()) };
        if old_font.is_invalid() {
            Err(FontExtractorError::WinApi {
                api_name: "SelectObject (select new font)".to_string(),
                source: WinError::from_win32(),
            })
        } else {
            Ok(Self {
                dc,
                old_font: Some(old_font),
                _marker: PhantomData,
            })
        }
    }
}
impl<'dc> Drop for FontSelector<'dc> {
    fn drop(&mut self) {
        if let Some(old_font_handle) = self.old_font {
            let _ = unsafe { SelectObject(self.dc.get(), old_font_handle) };
        }
    }
}

/// --- main 関数 ---
fn main() -> Result<(), FontExtractorError> {
    // --- コマンドライン引数の解析 ---
    let args = Args::parse();
    let font_name = &args.font_name;
    println!("Extracting font data for: {}", font_name);

    // --- リソースの確保 (RAII) ---
    let dc = SafeDC::new()?;
    let font = SafeFont::create(font_name)?;
    let _font_selector = FontSelector::select(&dc, &font)?;

    // --- フォントデータの取得 ---
    let data_size = unsafe { GetFontData(dc.get(), 0, 0, None, 0) };

    if data_size == GDI_ERROR as u32 {
        return Err(FontExtractorError::WinApi {
            api_name: "GetFontData (get size)".to_string(),
            source: WinError::from_win32(),
        });
    }
    if data_size == 0 {
        return Err(FontExtractorError::ZeroSizeFont {
            font_name: font_name.to_string(),
        });
    }
    println!("Font data size: {} bytes", data_size);

    let mut buffer: Vec<u8> = vec![0; data_size as usize];
    let bytes_written = unsafe {
        GetFontData(
            dc.get(),
            0,
            0,
            Some(buffer.as_mut_ptr() as *mut c_void),
            data_size,
        )
    };
    if bytes_written == GDI_ERROR as u32 {
        return Err(FontExtractorError::WinApi {
            api_name: "GetFontData (get data)".to_string(),
            source: WinError::from_win32(),
        });
    }
    if bytes_written != data_size {
        return Err(FontExtractorError::FontDataSizeMismatch {
            expected: data_size,
            got: bytes_written,
        });
    }

    // --- フォントデータの先頭でフォント種別を判定 ---
    let ext = if buffer.len() >= 4 {
        if &buffer[..4] == b"OTTO" {
            "otf"
        } else if &buffer[..4] == b"\x00\x01\x00\x00" {
            "ttf"
        } else if &buffer[..4] == b"ttcf" {
            "ttc"
        } else {
            "bin" // 不明な場合はデフォルトで bin 拡張子
        }
    } else {
        "bin"
    };

    // --- 出力パスの構築 ---
    // ユーザー指定のフォント名に既に拡張子がある場合は上書きします。
    let mut output_path = args.output_dir.join(font_name);
    output_path.set_extension(ext); // 拡張子を上書き

    let output_path_str = output_path.display().to_string();
    println!("Writing font data to: {}", output_path.display());

    // --- 保存先ディレクトリの作成 ---
    if let Some(parent_dir) = output_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| FontExtractorError::FileCreate {
            path: parent_dir.display().to_string(),
            source: e,
        })?;
    }

    // --- ファイルへの書き込み ---
    let mut file = fs::File::create(&output_path).map_err(|e| FontExtractorError::FileCreate {
        path: output_path_str.clone(),
        source: e,
    })?;
    file.write_all(&buffer)
        .map_err(|e| FontExtractorError::FileWrite {
            path: output_path_str,
            source: e,
        })?;

    println!("Font data extracted successfully!");

    // --- リソース解放 (変更なし、RAIIにより自動) ---
    Ok(())
}
