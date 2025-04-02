use clap::Parser; // clap を使うために追加
use std::ffi::{OsStr, c_void};
use std::fs;
use std::io::Write;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf; // PathBuf を使うために追加

// serde と toml は不要になったためコメントアウト (または削除)
// use serde::Deserialize;
// thiserror
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

// --- コマンドライン引数定義 (clap を使用) ---
#[derive(Parser, Debug)]
#[command(version, about = "Extracts font data from an installed font.", long_about = None)]
struct Args {
    /// Name of the font to extract (e.g., "Arial", "Times New Roman")
    #[arg(long)]
    font_name: String,

    /// Directory where the font file should be saved
    #[arg(long)]
    output_dir: PathBuf, // 保存先ディレクトリを PathBuf で受け取る
}

// --- カスタムエラー型定義 (toml 関連を削除) ---
#[derive(Error, Debug)]
pub enum FontExtractorError {
    // ConfigRead と ConfigParse を削除
    #[error("Windows API call '{api_name}' failed: {source}")]
    WinApi { api_name: String, source: WinError },
    #[error("Font '{font_name}' reported size 0 or could not be read.")]
    ZeroSizeFont { font_name: String },
    #[error("GetFontData reported unexpected size: expected {expected}, got {got}")]
    FontDataSizeMismatch { expected: u32, got: u32 },
    // FileCreate/FileWrite の path は String のまま (PathBuf.display().to_string() で渡す)
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

// --- Config 構造体は不要になったため削除 ---
// #[derive(Deserialize)]
// struct Config {
//     font_name: String,
//     output_filename: String, // ディレクトリ名に変更される
// }

// --- RAII ラッパー: SafeDC (変更なし) ---
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

// --- RAII ラッパー: SafeFont (変更なし) ---
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

// --- RAII ラッパー: FontSelector (変更なし) ---
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

// --- main 関数 (設定ファイル読み込み部分を clap に変更) ---
fn main() -> Result<(), FontExtractorError> {
    // --- コマンドライン引数の解析 ---
    let args = Args::parse(); // clap で引数を解析

    // --- 変数の設定 ---
    let font_name = &args.font_name; // コマンドライン引数からフォント名を取得
    // 保存先ファイルパスを生成: (保存先ディレクトリ名)/(フォント名)
    // 例: --output-dir C:\fonts --font-name arial.ttf -> C:\fonts\arial.ttf
    // 例: --output-dir ./out --font-name "My Font" -> ./out/My Font
    let output_path = args.output_dir.join(font_name); // PathBuf の join を使用
    // エラー表示用に文字列化しておく
    let output_path_str = output_path.display().to_string();

    println!("Extracting font data for: {}", font_name);

    // --- リソースの確保 (RAII) (変更なし) ---
    let dc = SafeDC::new()?;
    let font = SafeFont::create(font_name)?;
    let _font_selector = FontSelector::select(&dc, &font)?;

    // --- フォントデータの取得 (unsafe ブロックは最小限に) (変更なし) ---
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

    // --- ファイルへの書き込み (PathBuf を使用) ---
    println!("Writing font data to: {}", output_path.display()); // display() で表示

    // 親ディレクトリが存在しない場合に作成する
    if let Some(parent_dir) = output_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| FontExtractorError::FileCreate {
            // エラーメッセージには親ディレクトリのパスを表示
            path: parent_dir.display().to_string(),
            source: e,
        })?;
    }

    // ファイルを作成して書き込む
    let mut file = fs::File::create(&output_path).map_err(|e| FontExtractorError::FileCreate {
        path: output_path_str.clone(), // エラー用に文字列化したパスを使用
        source: e,
    })?;
    file.write_all(&buffer)
        .map_err(|e| FontExtractorError::FileWrite {
            path: output_path_str, // エラー用に文字列化したパスを使用
            source: e,
        })?;

    println!("Font data extracted successfully!");

    // --- リソース解放 (変更なし、RAIIにより自動) ---
    Ok(())
}
