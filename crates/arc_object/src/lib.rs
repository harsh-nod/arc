//! ELF64 object file generation for AIR.

/// A compiled object file with machine code and symbols.
#[derive(Debug, Clone)]
pub struct ObjectFile {
    pub format: String,
    /// Raw machine code for the .text section.
    pub text: Vec<u8>,
    /// Symbol table: (name, offset_in_text).
    pub symbols: Vec<(String, usize)>,
}

impl ObjectFile {
    pub fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
            text: Vec::new(),
            symbols: Vec::new(),
        }
    }

    /// Create an ELF64 object file with the given text section and symbols.
    pub fn elf64(text: Vec<u8>, symbols: Vec<(String, usize)>) -> Self {
        Self {
            format: "elf64-x86-64".into(),
            text,
            symbols,
        }
    }

    /// Serialize to ELF64 format bytes.
    pub fn to_elf(&self) -> Vec<u8> {
        let mut out = Vec::new();

        // We build a minimal relocatable ELF64 with:
        //   Section 0: null
        //   Section 1: .text (machine code)
        //   Section 2: .symtab (symbol table)
        //   Section 3: .strtab (string table)
        //   Section 4: .shstrtab (section name string table)

        // Build string tables first
        let mut shstrtab = StringTable::new();
        let _null_shstr = shstrtab.add("");
        let text_name = shstrtab.add(".text");
        let symtab_name = shstrtab.add(".symtab");
        let strtab_name = shstrtab.add(".strtab");
        let shstrtab_name = shstrtab.add(".shstrtab");

        let mut strtab = StringTable::new();
        let _null_str = strtab.add("");
        let sym_name_offsets: Vec<u32> = self
            .symbols
            .iter()
            .map(|(name, _)| strtab.add(name))
            .collect();

        // ELF Header (64 bytes)
        let ehdr_size: u16 = 64;
        let shdr_size: u16 = 64;
        let num_sections: u16 = 5; // null, .text, .symtab, .strtab, .shstrtab
        let shstrtab_idx: u16 = 4;

        // Layout:
        // [0..64)       ELF header
        // [64..64+text_len)  .text
        // [text_end..]  .symtab, .strtab, .shstrtab, section headers

        let text_offset: u64 = ehdr_size as u64;
        let text_size: u64 = self.text.len() as u64;

        // .symtab entries: null + one per symbol
        let sym_entry_size: u64 = 24; // Elf64_Sym
        let num_syms = 1 + self.symbols.len() as u64;
        let symtab_size = num_syms * sym_entry_size;

        let symtab_offset = text_offset + text_size;
        let strtab_offset = symtab_offset + symtab_size;
        let strtab_size = strtab.data.len() as u64;
        let shstrtab_offset = strtab_offset + strtab_size;
        let shstrtab_size = shstrtab.data.len() as u64;
        let shdr_offset = shstrtab_offset + shstrtab_size;

        // === ELF Header ===
        out.extend_from_slice(&[0x7f, b'E', b'L', b'F']); // e_ident magic
        out.push(2); // EI_CLASS: ELFCLASS64
        out.push(1); // EI_DATA: ELFDATA2LSB
        out.push(1); // EI_VERSION
        out.push(0); // EI_OSABI: ELFOSABI_NONE
        out.extend_from_slice(&[0; 8]); // EI_ABIVERSION + padding
        out.extend_from_slice(&1u16.to_le_bytes()); // e_type: ET_REL
        out.extend_from_slice(&0x3Eu16.to_le_bytes()); // e_machine: EM_X86_64
        out.extend_from_slice(&1u32.to_le_bytes()); // e_version
        out.extend_from_slice(&0u64.to_le_bytes()); // e_entry
        out.extend_from_slice(&0u64.to_le_bytes()); // e_phoff
        out.extend_from_slice(&shdr_offset.to_le_bytes()); // e_shoff
        out.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        out.extend_from_slice(&ehdr_size.to_le_bytes()); // e_ehsize
        out.extend_from_slice(&0u16.to_le_bytes()); // e_phentsize
        out.extend_from_slice(&0u16.to_le_bytes()); // e_phnum
        out.extend_from_slice(&shdr_size.to_le_bytes()); // e_shentsize
        out.extend_from_slice(&num_sections.to_le_bytes()); // e_shnum
        out.extend_from_slice(&shstrtab_idx.to_le_bytes()); // e_shstrndx

        assert_eq!(out.len(), ehdr_size as usize);

        // === .text ===
        out.extend_from_slice(&self.text);

        // === .symtab ===
        // Null symbol entry
        out.extend_from_slice(&[0u8; 24]);
        // Symbol entries
        for (i, (_name, offset)) in self.symbols.iter().enumerate() {
            let st_name = sym_name_offsets[i];
            out.extend_from_slice(&st_name.to_le_bytes()); // st_name
            let st_info: u8 = (1 << 4) | 2; // STB_GLOBAL | STT_FUNC
            out.push(st_info); // st_info
            out.push(0); // st_other
            out.extend_from_slice(&1u16.to_le_bytes()); // st_shndx (.text = section 1)
            out.extend_from_slice(&(*offset as u64).to_le_bytes()); // st_value
            out.extend_from_slice(&0u64.to_le_bytes()); // st_size (unknown)
        }

        // === .strtab ===
        out.extend_from_slice(&strtab.data);

        // === .shstrtab ===
        out.extend_from_slice(&shstrtab.data);

        // === Section Headers ===
        // Section 0: null
        out.extend_from_slice(&[0u8; 64]);

        // Section 1: .text
        write_shdr(
            &mut out,
            SectionHeader {
                name: text_name,
                section_type: 1, // SHT_PROGBITS
                flags: 6,        // SHF_ALLOC | SHF_EXECINSTR
                offset: text_offset,
                size: text_size,
                link: 0,
                info: 0,
                align: 16,
                entry_size: 0,
            },
        );

        // Section 2: .symtab
        write_shdr(
            &mut out,
            SectionHeader {
                name: symtab_name,
                section_type: 2, // SHT_SYMTAB
                flags: 0,
                offset: symtab_offset,
                size: symtab_size,
                link: 3, // sh_link -> .strtab
                info: 1, // sh_info -> first non-local symbol
                align: 8,
                entry_size: sym_entry_size,
            },
        );

        // Section 3: .strtab
        write_shdr(
            &mut out,
            SectionHeader {
                name: strtab_name,
                section_type: 3, // SHT_STRTAB
                flags: 0,
                offset: strtab_offset,
                size: strtab_size,
                link: 0,
                info: 0,
                align: 1,
                entry_size: 0,
            },
        );

        // Section 4: .shstrtab
        write_shdr(
            &mut out,
            SectionHeader {
                name: shstrtab_name,
                section_type: 3, // SHT_STRTAB
                flags: 0,
                offset: shstrtab_offset,
                size: shstrtab_size,
                link: 0,
                info: 0,
                align: 1,
                entry_size: 0,
            },
        );

        out
    }
}

struct SectionHeader {
    name: u32,
    section_type: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entry_size: u64,
}

fn write_shdr(out: &mut Vec<u8>, shdr: SectionHeader) {
    out.extend_from_slice(&shdr.name.to_le_bytes());
    out.extend_from_slice(&shdr.section_type.to_le_bytes());
    out.extend_from_slice(&shdr.flags.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
    out.extend_from_slice(&shdr.offset.to_le_bytes());
    out.extend_from_slice(&shdr.size.to_le_bytes());
    out.extend_from_slice(&shdr.link.to_le_bytes());
    out.extend_from_slice(&shdr.info.to_le_bytes());
    out.extend_from_slice(&shdr.align.to_le_bytes());
    out.extend_from_slice(&shdr.entry_size.to_le_bytes());
}

/// Simple null-terminated string table builder.
struct StringTable {
    data: Vec<u8>,
}

impl StringTable {
    fn new() -> Self {
        Self { data: vec![0] } // Start with null byte
    }

    fn add(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elf64_magic_and_header() {
        let obj = ObjectFile::elf64(vec![0xC3], vec![("_start".to_string(), 0)]);
        let elf = obj.to_elf();

        // ELF magic
        assert_eq!(&elf[0..4], b"\x7fELF");
        // 64-bit, little-endian
        assert_eq!(elf[4], 2);
        assert_eq!(elf[5], 1);
        // ET_REL
        assert_eq!(u16::from_le_bytes([elf[16], elf[17]]), 1);
        // EM_X86_64
        assert_eq!(u16::from_le_bytes([elf[18], elf[19]]), 0x3E);
    }

    #[test]
    fn elf64_contains_text() {
        let code = vec![0x55, 0xC3]; // push rbp; ret
        let obj = ObjectFile::elf64(code.clone(), vec![]);
        let elf = obj.to_elf();

        // .text starts at offset 64 (right after ELF header)
        assert_eq!(&elf[64..66], &code);
    }

    #[test]
    fn elf64_section_count() {
        let obj = ObjectFile::elf64(vec![0xC3], vec![]);
        let elf = obj.to_elf();

        // e_shnum at offset 60
        let shnum = u16::from_le_bytes([elf[60], elf[61]]);
        assert_eq!(shnum, 5); // null, .text, .symtab, .strtab, .shstrtab
    }

    #[test]
    fn elf64_symbol_table() {
        let obj = ObjectFile::elf64(
            vec![0x55, 0xC3],
            vec![("main".to_string(), 0), ("helper".to_string(), 1)],
        );
        let elf = obj.to_elf();
        // Should contain the symbol names in .strtab
        let elf_str = String::from_utf8_lossy(&elf);
        assert!(elf_str.contains("main"));
        assert!(elf_str.contains("helper"));
    }
}
