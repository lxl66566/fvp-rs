pub mod error;
use std::io::{self, Read, Seek, SeekFrom, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use encoding_rs::SHIFT_JIS;
pub use error::*;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    // 文件名在名称段的偏移量
    pub name_offset: u32,
}

impl Entry {
    /// 从给定的 reader 中提取此 Entry 对应的数据并写入 writer。
    ///
    /// # Arguments
    /// * `reader` - 源数据流 (必须支持 Seek 以便跳转到 offset)
    /// * `writer` - 目标写入流
    pub fn extract_from<R, W>(&self, reader: &mut R, writer: &mut W) -> Result<u64>
    where
        R: Read + Seek,
        W: Write,
    {
        reader.seek(SeekFrom::Start(self.offset as u64))?;
        // 使用 take 限制读取长度，防止读过界，同时利用 io::copy 的优化
        let mut chunk = reader.by_ref().take(self.size as u64);
        let copied = io::copy(&mut chunk, writer)?;
        Ok(copied)
    }
}

pub struct FvpReader<R> {
    inner: R,
    entries: Vec<Entry>,
}

impl<R: Read + Seek> FvpReader<R> {
    /// 打开并解析归档头部
    pub fn new(mut inner: R) -> Result<Self> {
        // 1. 读取头部
        let file_count = inner.read_u32::<LittleEndian>()? as usize;
        let _file_names_size = inner.read_u32::<LittleEndian>()?;

        let table_size = (file_count * 12) as u64;
        let file_names_start = 8 + table_size;

        // 2. 读取文件表
        struct RawEntry {
            name_offset: u32,
            offset: u32,
            size: u32,
        }

        let mut raw_entries = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            raw_entries.push(RawEntry {
                name_offset: inner.read_u32::<LittleEndian>()?,
                offset: inner.read_u32::<LittleEndian>()?,
                size: inner.read_u32::<LittleEndian>()?,
            });
        }

        // 3. 读取文件名
        let mut entries = Vec::with_capacity(file_count);
        for raw in raw_entries {
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

        Ok(Self { inner, entries })
    }

    /// 获取条目列表的引用
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 将 Reader 拆解为元数据列表和底层 IO 流。
    pub fn into_parts(self) -> (Vec<Entry>, R) {
        (self.entries, self.inner)
    }

    /// 传统接口：通过索引提取文件
    pub fn extract_file<W: Write>(&mut self, index: usize, writer: &mut W) -> Result<usize> {
        if let Some(entry) = self.entries.get(index) {
            let entry = entry.clone();
            let size = entry.extract_from(&mut self.inner, writer)?;
            Ok(size as usize)
        } else {
            Err(FvpError::FormatError(format!(
                "Index {} out of bounds",
                index
            )))
        }
    }
}

/// 写入器：用于创建新的归档
///
/// 采用 Builder 模式。由于文件格式要求在头部写入所有文件名，
/// 所以必须在开始写入数据前提供所有文件名。
pub struct FvpBuilder<W> {
    inner: W,
    entries: Vec<Entry>,
    current_file_index: usize,
    #[allow(dead_code)]
    names_start_pos: u64,
}

impl<W: Write + Seek> FvpBuilder<W> {
    /// 创建一个新的 Builder。
    ///
    /// `file_names`: 必须按顺序提供所有将要写入的文件名。
    /// 库会自动处理 Shift-JIS 编码和头部占位符的写入。
    pub fn new(mut inner: W, file_names: &[&str]) -> Result<Self> {
        let file_count = file_names.len();

        // 1. 预处理文件名，计算长度
        let mut encoded_names = Vec::new();
        let mut total_names_len = 0;

        for name in file_names {
            let (cow, _, had_errors) = SHIFT_JIS.encode(name);
            if had_errors {
                return Err(FvpError::EncodingError);
            }
            // 加上 null terminator
            let mut bytes = cow.into_owned();
            bytes.push(0);
            total_names_len += bytes.len();
            encoded_names.push(bytes);
        }

        // 2. 写入 Header
        inner.write_u32::<LittleEndian>(file_count as u32)?;
        inner.write_u32::<LittleEndian>(total_names_len as u32)?;

        // 3. 写入 Table 占位符 (全0)
        // Table size = count * 12
        let table_size = file_count * 12;
        let _table_start_pos = inner.stream_position()?;
        io::copy(&mut io::repeat(0).take(table_size as u64), &mut inner)?;

        // 4. 写入文件名区域
        let names_start_pos = inner.stream_position()?;
        let mut entries = Vec::with_capacity(file_count);
        let mut current_name_offset = 0;

        for (i, bytes) in encoded_names.iter().enumerate() {
            inner.write_all(bytes)?;

            entries.push(Entry {
                name: file_names[i].to_string(),
                offset: 0, // 稍后填充
                size: 0,   // 稍后填充
                name_offset: current_name_offset as u32,
            });

            current_name_offset += bytes.len();
        }

        Ok(Self {
            inner,
            entries,
            current_file_index: 0,
            names_start_pos,
        })
    }

    /// 写入下一个文件的数据。
    /// 必须按照 `new` 中提供的文件名顺序调用此方法。
    pub fn write_file<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        if self.current_file_index >= self.entries.len() {
            return Err(FvpError::CountMismatch {
                expected: self.entries.len(),
                actual: self.current_file_index + 1,
            });
        }

        let start_pos = self.inner.stream_position()?;
        let size = io::copy(reader, &mut self.inner)?;

        // 更新当前条目的元数据
        let entry = &mut self.entries[self.current_file_index];
        entry.offset = start_pos as u32;
        entry.size = size as u32;

        self.current_file_index += 1;
        Self::finish_check(self);
        Ok(())
    }

    /// 内部辅助：检查是否写完，
    /// 如果写完可以做一些清理（这里主要是为了逻辑完整性）
    fn finish_check(&self) {}

    /// 完成归档。
    /// 回跳到文件开头，填充 Table 中的 offset 和 size。
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

        // 跳回文件末尾（可选，符合直觉）
        self.inner.seek(SeekFrom::End(0))?;

        Ok(())
    }
}

/// 辅助函数：读取 Shift-JIS 字符串直到 \0
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
