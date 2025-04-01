use std::ffi::{OsStr, c_void};
use std::fs;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::marker::PhantomData; // FontSelector でライフタイムを扱うために追加

// serde
use serde::Deserialize;
// thiserror
use thiserror::Error;

// Windows API 関連
use windows::{
    core::{Error as WinError, PCWSTR}, // windows::core::Error を WinError としてインポート
    Win32::
        Graphics::Gdi::{
            CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
            DEFAULT_QUALITY, DeleteDC, DeleteObject, FF_DONTCARE, FW_NORMAL, GDI_ERROR,
            GetFontData, SelectObject, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, // SelectObject の戻り型チェック用に追加の可能性 (今回はis_invalidで十分)
        }
    ,
};

// --- カスタムエラー型定義 ---
#[derive(Error, Debug)]
pub enum FontExtractorError {
    #[error("Failed to read config file '{path}': {source}")]
    ConfigRead { path: String, source: std::io::Error },
    #[error("Failed to parse config file '{path}': {source}")]
    ConfigParse { path: String, source: toml::de::Error },
    #[error("Windows API call '{api_name}' failed: {source}")]
    WinApi { api_name: String, source: WinError }, // windows::core::Error を使う
    #[error("Font '{font_name}' reported size 0 or could not be read.")]
    ZeroSizeFont { font_name: String },
    #[error("GetFontData reported unexpected size: expected {expected}, got {got}")]
    FontDataSizeMismatch { expected: u32, got: u32 },
    #[error("Failed to create output file '{path}': {source}")]
    FileCreate { path: String, source: std::io::Error },
    #[error("Failed to write to output file '{path}': {source}")]
    FileWrite { path: String, source: std::io::Error },
    // try_into が必要な場合のエラー (今回は不要になったが例として残す)
    // #[error("Failed to convert font weight constant: {value}")]
    // InvalidFontWeight { value: i32 },
}

// --- 設定ファイルのための構造体 ---
#[derive(Deserialize)]
struct Config {
    font_name: String,
    output_filename: String,
}

// --- RAII ラッパー: HDC ---
struct SafeDC(HDC);

impl SafeDC {
    fn new() -> Result<Self, FontExtractorError> {
        // unsafe: CreateCompatibleDC は外部関数呼び出し
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
            // unsafe: DeleteDC は外部関数呼び出し
            let _ = unsafe { DeleteDC(self.0) }; // Drop 中のエラーは通常無視
        }
    }
}

// --- RAII ラッパー: HFONT ---
struct SafeFont(HFONT);

impl SafeFont {
    fn create(font_name: &str) -> Result<Self, FontExtractorError> {
        let font_name_wide: Vec<u16> = OsStr::new(font_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let pcwstr_font_name = PCWSTR(font_name_wide.as_ptr());

        // unsafe: CreateFontW は外部関数呼び出し
        let font = unsafe {
            CreateFontW(
                0,                        // nHeight
                0,                        // nWidth
                0,                        // nEscapement
                0,                        // nOrientation
                FW_NORMAL.0.try_into().unwrap(), // fnWeight
                0,                        // fdwItalic
                0,                        // fdwUnderline
                0,                        // fdwStrikeOut
                DEFAULT_CHARSET.0.into(), // fdwCharSet (u8 -> u32)
                OUT_DEFAULT_PRECIS.0.into(), // fdwOutputPrecision (u8 -> u32)
                CLIP_DEFAULT_PRECIS.0.into(), // fdwClipPrecision (u8 -> u32)
                DEFAULT_QUALITY.0.into(), // fdwQuality (u8 -> u32)
                (DEFAULT_PITCH.0 | FF_DONTCARE.0).into(), // fdwPitchAndFamily (u8 -> u32)
                pcwstr_font_name,         // lpszFace
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
            // unsafe: DeleteObject は外部関数呼び出し
            // HFONT は HGDIOBJ に変換可能
            let _ = unsafe { DeleteObject(self.0) }; // Drop 中のエラーは通常無視
        }
    }
}


// --- RAII ラッパー: FontSelector (DCにフォントを選択し、Dropで元に戻す) ---
struct FontSelector<'dc> {
    dc: &'dc SafeDC,
    old_font: Option<HGDIOBJ>,
    _marker: PhantomData<&'dc ()>, // dc のライフタイムを正しく紐付ける
}

impl<'dc> FontSelector<'dc> {
    fn select(dc: &'dc SafeDC, font: &SafeFont) -> Result<Self, FontExtractorError> {
        // unsafe: SelectObject は外部関数呼び出し
        let old_font = unsafe { SelectObject(dc.get(), font.get()) };

        // SelectObject の戻り値は NULL または HGDI_ERROR(-1) で失敗を示すことが多い
        // is_invalid() は 0 (NULL) を無効とみなすため、これでチェックできるはず
        // GDI オブジェクトの種類によっては 0 が有効な場合もあるが、フォント選択では問題ないはず
        if old_font.is_invalid() {
            // エラーが発生した場合、GetFontData はおそらく失敗する
            Err(FontExtractorError::WinApi {
                api_name: "SelectObject (select new font)".to_string(),
                source: WinError::from_win32(), // 直前のAPIエラーを取得
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
            // unsafe: SelectObject は外部関数呼び出し
            // Drop 中のエラーは通常無視するかログ記録
            let _ = unsafe { SelectObject(self.dc.get(), old_font_handle) };
        }
    }
}

// --- main 関数 ---
fn main() -> Result<(), FontExtractorError> {
    // --- 設定ファイルの読み込み ---
    let config_path = "config.toml";
    let config_content = fs::read_to_string(config_path)
        .map_err(|e| FontExtractorError::ConfigRead { path: config_path.to_string(), source: e })?;
    let config: Config = toml::from_str(&config_content)
        .map_err(|e| FontExtractorError::ConfigParse { path: config_path.to_string(), source: e })?;

    let font_name = &config.font_name;
    let output_filename = &config.output_filename;

    println!("Extracting font data for: {}", font_name);

    // --- リソースの確保 (RAII) ---
    let dc = SafeDC::new()?;
    let font = SafeFont::create(font_name)?;
    let _font_selector = FontSelector::select(&dc, &font)?; // Drop で元に戻る

    // --- フォントデータの取得 (unsafe ブロックは最小限に) ---
    let data_size = unsafe {
        // GetFontData は外部関数呼び出し
        GetFontData(dc.get(), 0, 0, None, 0)
    };

    // GDI_ERROR は windows クレート (0.5x) では u32 なのでキャスト不要
    if data_size == GDI_ERROR as u32 {
        return Err(FontExtractorError::WinApi {
            api_name: "GetFontData (get size)".to_string(),
            source: WinError::from_win32(), // 直前のAPIエラーを取得
        });
    }
    if data_size == 0 {
        // 特定のフォント (システムフォントなど) はファイル実体を持たないことがある
        return Err(FontExtractorError::ZeroSizeFont { font_name: font_name.to_string() });
    }

    println!("Font data size: {} bytes", data_size);

    let mut buffer: Vec<u8> = vec![0; data_size as usize];

    let bytes_written = unsafe {
        // GetFontData は外部関数呼び出し
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

    // --- ファイルへの書き込み ---
    println!("Writing font data to: {}", output_filename);
    let path = Path::new(output_filename);
    let mut file = std::fs::File::create(path).map_err(|e| FontExtractorError::FileCreate {
        path: output_filename.to_string(),
        source: e,
    })?;
    file.write_all(&buffer).map_err(|e| FontExtractorError::FileWrite {
        path: output_filename.to_string(),
        source: e,
    })?;

    println!("Font data extracted successfully!");

    // --- リソース解放 ---
    // SafeDC, SafeFont, FontSelector がスコープを抜ける際に Drop が呼ばれ、
    // 自動的に DeleteDC, DeleteObject, SelectObject(元に戻す) が実行される。
    // 手動での解放処理は不要になる。

    Ok(())
}