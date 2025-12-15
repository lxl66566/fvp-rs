pub mod error;
use std::{
    cell::RefCell,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use encoding_rs::SHIFT_JIS;
pub use error::*;

/// 归档中的单个文件条目（元数据）
#[derive(Debug, Clone)]
pub struct Entry {
    /// 归档中存储的文件名（可能是去除了后缀的）
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub name_offset: u32,
}

/// 文件格式检测
struct MagicDetector;

impl MagicDetector {
    const OGG_MAGIC: &'static [u8] = b"OggS";

    /// 检查给定的 buffer 是否匹配已知格式，并返回建议的扩展名
    fn suggest_extension(header: &[u8]) -> Option<&'static str> {
        if header.starts_with(Self::OGG_MAGIC) {
            return Some(".ogg");
        }
        // 在这里可以方便地扩展其他格式，例如：
        // if header.starts_with(b"RIFF") { return Some(".wav"); }
        None
    }

    /// 检查文件名是否应该被剥离后缀（用于打包时）
    fn should_strip_extension(filename: &str) -> bool {
        // 当前逻辑：只要是 .ogg 结尾就剥离，为了保持对偶性
        filename.to_lowercase().ends_with(".ogg")
    }

    /// 剥离后缀
    fn strip_extension(filename: &str) -> String {
        if let Some(stem) = Path::new(filename).file_stem()
            && let Some(s) = stem.to_str()
        {
            return s.to_string();
        }
        filename.to_string()
    }
}

/// FVP 归档读取器。
///
/// 使用 `RefCell` 实现了内部可变性，允许在持有 `&FvpReader`
/// 引用的同时读取数据。
pub struct FvpReader<R> {
    // 使用 RefCell 允许我们在 &self 方法中修改流的游标 (Seek/Read)
    inner: RefCell<R>,
    entries: Vec<Entry>,
    // 如果为 true，则不进行任何智能文件名处理（不添加 .ogg 后缀）。默认 false。
    raw_mode: bool,
}

impl<R: Read + Seek> FvpReader<R> {
    /// 打开并解析归档。默认 `raw_mode = false`（智能模式）。
    pub fn new(mut inner: R) -> Result<Self> {
        // 1. 读取头部
        let file_count = inner.read_u32::<LittleEndian>()? as usize;
        let _file_names_size = inner.read_u32::<LittleEndian>()?;

        let table_size = (file_count * 12) as u64;
        let file_names_start = 8 + table_size;

        // 2. 读取文件表 (Raw)
        struct RawTableEntry {
            name_offset: u32,
            offset: u32,
            size: u32,
        }

        let mut raw_table = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            raw_table.push(RawTableEntry {
                name_offset: inner.read_u32::<LittleEndian>()?,
                offset: inner.read_u32::<LittleEndian>()?,
                size: inner.read_u32::<LittleEndian>()?,
            });
        }

        // 3. 读取文件名并构建 Entry
        let mut entries = Vec::with_capacity(file_count);
        for raw in raw_table {
            let name_abs_pos = file_names_start + raw.name_offset as u64;
            inner.seek(SeekFrom::Start(name_abs_pos))?;
            let name = read_sjis_string(&mut inner)?;

            entries.push(Entry {
                name,
                offset: raw.offset,
                size: raw.size,
                name_offset: raw.name_offset,
            });
        }

        Ok(Self {
            inner: RefCell::new(inner),
            entries,
            raw_mode: false,
        })
    }

    /// 设置是否开启 Raw 模式。
    /// - `true`: 直接返回归档内的文件名，不进行魔数检测。
    /// - `false`: 根据文件头内容智能添加后缀（如 .ogg）。
    pub fn with_raw_mode(mut self, raw_mode: bool) -> Self {
        self.raw_mode = raw_mode;
        self
    }

    /// 获取文件条目列表。
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 获取指定 Entry 的“解析后文件名”。
    ///
    /// 如果 `raw_mode` 为 false，此方法会 Seek 到文件位置并读取前几个字节
    /// 来判断是否需要添加后缀。
    pub fn resolve_name(&self, entry: &Entry) -> Result<String> {
        if self.raw_mode {
            return Ok(entry.name.clone());
        }

        // 智能模式：Peek 文件头
        let mut reader = self.inner.borrow_mut();
        reader.seek(SeekFrom::Start(entry.offset as u64))?;

        let mut magic_buf = [0u8; 4];
        // 处理文件极小的情况
        let read_len = reader.read(&mut magic_buf)?;

        if let Some(ext) = MagicDetector::suggest_extension(&magic_buf[..read_len]) {
            // 如果文件名本身没后缀，或者后缀不匹配，才添加
            if !entry.name.to_lowercase().ends_with(ext) {
                return Ok(format!("{}{}", entry.name, ext));
            }
        }

        Ok(entry.name.clone())
    }

    /// 将指定条目的数据提取到 writer 中。
    ///
    /// 这个方法获取 `&self` 而不是 `&mut self`，这得益于内部 `RefCell`。
    /// 这允许你在迭代 `entries()` (持有 &self) 的同时调用此方法。
    pub fn extract_entry<W: Write>(&self, entry: &Entry, writer: &mut W) -> Result<u64> {
        let mut reader = self.inner.borrow_mut();
        reader.seek(SeekFrom::Start(entry.offset as u64))?;

        let mut chunk = reader.by_ref().take(entry.size as u64);
        let copied = io::copy(&mut chunk, writer)?;
        Ok(copied)
    }
}

#[allow(dead_code)]
pub struct FvpBuilder<W> {
    inner: W,
    entries: Vec<Entry>,
    current_file_index: usize,
    // 如果为 true，则不进行任何文件名处理（不添加 .ogg 后缀）。默认 false。
    raw_mode: bool,
}

impl<W: Write + Seek> FvpBuilder<W> {
    /// 创建一个新的 Builder。
    ///
    /// `file_names`: 输入的文件名列表。
    /// `raw_mode`: 如果为 false，会自动检测并移除 .ogg 等后缀再写入文件表。
    pub fn new(mut inner: W, file_names: &[&str], raw_mode: bool) -> Result<Self> {
        let file_count = file_names.len();
        let mut entries = Vec::with_capacity(file_count);
        let mut encoded_names_buffer = Vec::new();

        let mut current_name_offset = 0;

        // 1. 预处理文件名 (处理 Raw Mode 和 Shift-JIS)
        for name in file_names {
            // Raw Mode 处理：是否剥离后缀
            let stored_name_str = if !raw_mode && MagicDetector::should_strip_extension(name) {
                MagicDetector::strip_extension(name)
            } else {
                name.to_string()
            };

            // 编码
            let (cow, _, had_errors) = SHIFT_JIS.encode(&stored_name_str);
            if had_errors {
                return Err(FvpError::EncodingError);
            }
            let mut bytes = cow.into_owned();
            bytes.push(0); // Null terminator

            entries.push(Entry {
                name: stored_name_str, // 记录存储在归档里的名字
                offset: 0,
                size: 0,
                name_offset: current_name_offset as u32,
            });

            current_name_offset += bytes.len();
            encoded_names_buffer.extend_from_slice(&bytes);
        }

        // 2. 写入 Header
        inner.write_u32::<LittleEndian>(file_count as u32)?;
        inner.write_u32::<LittleEndian>(encoded_names_buffer.len() as u32)?;

        // 3. 写入 Table 占位符 (Count * 12 bytes)
        let table_size = file_count * 12;
        io::copy(&mut io::repeat(0).take(table_size as u64), &mut inner)?;

        // 4. 写入所有文件名
        inner.write_all(&encoded_names_buffer)?;

        Ok(Self {
            inner,
            entries,
            current_file_index: 0,
            raw_mode,
        })
    }

    /// 写入下一个文件的数据。
    /// 注意：输入流 `reader` 不需要 Seek，支持流式写入。
    pub fn write_file<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        if self.current_file_index >= self.entries.len() {
            return Err(FvpError::CountMismatch {
                expected: self.entries.len(),
                actual: self.current_file_index + 1,
            });
        }

        let start_pos = self.inner.stream_position()?;
        let size = io::copy(reader, &mut self.inner)?;

        // 更新当前条目的 offset 和 size
        let entry = &mut self.entries[self.current_file_index];
        entry.offset = start_pos as u32;
        entry.size = size as u32;

        self.current_file_index += 1;
        Ok(())
    }

    /// 完成归档，回写文件表。
    pub fn finish(&mut self) -> Result<()> {
        if self.current_file_index != self.entries.len() {
            return Err(FvpError::CountMismatch {
                expected: self.entries.len(),
                actual: self.current_file_index,
            });
        }

        // 回跳到 Table 开始位置 (Header 是 8 字节)
        self.inner.seek(SeekFrom::Start(8))?;

        for entry in &self.entries {
            self.inner.write_u32::<LittleEndian>(entry.name_offset)?;
            self.inner.write_u32::<LittleEndian>(entry.offset)?;
            self.inner.write_u32::<LittleEndian>(entry.size)?;
        }

        // 跳回末尾
        self.inner.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

fn read_sjis_string<R: Read>(reader: &mut R) -> Result<String> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        reader.read_exact(&mut buf)?;
        if buf[0] == 0 {
            break;
        }
        bytes.push(buf[0]);
    }
    let (cow, _, had_errors) = SHIFT_JIS.decode(&bytes);
    if had_errors {
        Err(FvpError::DecodingError)
    } else {
        Ok(cow.into_owned())
    }
}

/// 解包入口函数
pub fn extract(input_file: impl AsRef<Path>, output_dir: impl AsRef<Path>) -> Result<()> {
    let file = File::open(&input_file)?;
    let reader = FvpReader::new(BufReader::new(file))?;

    fs::create_dir_all(&output_dir)?;

    let entries = reader.entries();
    println!("Extracting {} files...", entries.len());

    for (i, entry) in entries.iter().enumerate() {
        let file_name = reader.resolve_name(entry)?;
        let target_path = output_dir.as_ref().join(&file_name);

        println!(
            "[{}/{}] {} -> {:?}",
            i + 1,
            entries.len(),
            entry.name,
            file_name
        );

        let mut target_file = File::create(target_path)?;
        reader.extract_entry(entry, &mut target_file)?;
    }

    Ok(())
}

/// 打包入口函数
pub fn pack(input_dir: impl AsRef<Path>, output_file: impl AsRef<Path>) -> Result<()> {
    let mut paths: Vec<_> = fs::read_dir(input_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();

    // 保证顺序一致
    paths.sort();

    let file_names: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();

    let file_names_ref: Vec<&str> = file_names.iter().map(|s| s.as_str()).collect();

    let file = File::create(output_file)?;
    // 默认 raw_mode = false，会自动剥离 .ogg 后缀
    let mut builder = FvpBuilder::new(BufWriter::new(file), &file_names_ref, false)?;

    for path in &paths {
        println!("Writing {:?}...", path.file_name().unwrap_or_default());
        let mut input = File::open(path)?;
        builder.write_file(&mut input)?;
    }

    builder.finish()?;
    println!("Done.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_extract() {
        let temp_dir = tempdir().unwrap();
        let input_file = temp_dir.path().join("input.bin");
        fs::write(&input_file, include_bytes!("../test_assets/test.bin")).unwrap();
        let output_dir = temp_dir.path().join("output");
        extract(&input_file, &output_dir).unwrap();

        assert_eq!(fs::read_to_string(output_dir.join("1.txt")).unwrap(), "111");
        assert_eq!(fs::read_to_string(output_dir.join("2.txt")).unwrap(), "222");
    }

    #[test]
    fn test_pack() {
        let temp_dir = tempdir().unwrap();
        let input_dir = temp_dir.path().join("input");
        fs::create_dir(&input_dir).unwrap();
        fs::write(input_dir.join("1.txt"), "111").unwrap();
        fs::write(input_dir.join("2.txt"), "222").unwrap();

        let output_file = temp_dir.path().join("output.bin");
        pack(&input_dir, &output_file).unwrap();

        assert_eq!(
            fs::read(output_file).unwrap(),
            include_bytes!("../test_assets/test.bin")
        );
    }

    fn print_dir(dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            println!("file in dir: {:?}", entry.path());
        }
    }

    #[test]
    fn test_resolve_name() {
        let temp_dir = tempdir().unwrap();
        let input_file = temp_dir.path().join("input.bin");
        fs::write(&input_file, include_bytes!("../test_assets/test2.bin")).unwrap();
        let output_dir = temp_dir.path().join("output");
        extract(input_file, &output_dir).unwrap();
        print_dir(&output_dir);

        let output_file = output_dir.join("none.ogg");
        assert!(output_file.exists());

        fs::rename(&output_file, output_file.with_extension("")).unwrap();
        let pack_output = temp_dir.path().join("output.bin");
        pack(&output_dir, &pack_output).unwrap();

        assert_eq!(
            fs::read(pack_output).unwrap(),
            include_bytes!("../test_assets/test2.bin")
        );
    }
}
