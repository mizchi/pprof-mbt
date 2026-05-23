//! [`FrameResolver`] implementation for samply's RVA-based frame table.
//!
//! samply (the Linux/macOS native sampler) does not pre-resolve symbols
//! into `funcTable`. Instead, `frameTable.address[i]` carries the raw
//! virtual address inside the owning lib, and a separate `.syms.json`
//! sidecar maps each lib's RVA ranges to symbols + inline chains.
//!
//! [`SamplySyms`] parses that sidecar, indexes it by `debugName`, and
//! exposes a [`FrameResolver`] that binary-searches per-frame addresses
//! into the enclosing symbol and unfolds inline frames leaf-first to
//! match pprof's `Location.line[]` ordering.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{FirefoxProfile, FrameResolver, ResolvedFrame, Thread};

/// Parsed `.syms.json` sidecar produced by samply with
/// `--unstable-presymbolicate`.
#[derive(Debug, Deserialize)]
pub struct SamplySyms {
    /// Per-lib symbol tables. Each entry's `symbol_table` is sorted by
    /// `rva` at parse time.
    pub data: Vec<LibSyms>,
    /// Shared string pool; every `symbol` / `function` / `file` index
    /// in this file points into here.
    pub string_table: Vec<String>,
}

/// Symbols for one loaded lib.
#[derive(Debug, Deserialize)]
pub struct LibSyms {
    /// `Lib.debugName` to match against `FirefoxProfile.libs[].debug_name`.
    pub debug_name: String,
    /// Symbols inside this lib. Sorted by `rva` after [`SamplySyms::load`].
    pub symbol_table: Vec<SymEntry>,
}

/// One symbol in a [`LibSyms`].
#[derive(Debug, Deserialize)]
pub struct SymEntry {
    /// Lib-relative virtual address (start of the symbol).
    pub rva: u64,
    /// Byte size of the symbol's code range.
    pub size: u64,
    /// `string_table` index for the symbol's mangled name.
    pub symbol: i64,
    /// Inline chain (outer → inner). Empty when no inline info exists.
    #[serde(default)]
    pub frames: Vec<SymFrame>,
}

/// One inline frame inside a symbol.
#[derive(Debug, Deserialize)]
pub struct SymFrame {
    /// `string_table` index for the inlined function's name.
    pub function: i64,
    /// `string_table` index for the inlined source file, if known.
    #[serde(default)]
    pub file: Option<i64>,
    /// Source line, if known.
    #[serde(default)]
    pub line: Option<i64>,
}

impl SamplySyms {
    /// Parse a `.syms.json` payload and sort each lib's `symbol_table` by
    /// `rva` so subsequent lookups can binary-search.
    pub fn load(bytes: &[u8]) -> Result<Self> {
        let mut me: SamplySyms = serde_json::from_slice(bytes)
            .context("parsing samply .syms.json sidecar")?;
        for lib in &mut me.data {
            lib.symbol_table.sort_by_key(|e| e.rva);
        }
        Ok(me)
    }

    /// Build the [`FrameResolver`] that pairs this sidecar with a
    /// samply-produced [`FirefoxProfile`].
    pub fn into_resolver(self) -> SamplySymsResolver {
        let mut libs = HashMap::with_capacity(self.data.len());
        for lib in self.data {
            libs.insert(lib.debug_name.clone(), lib);
        }
        SamplySymsResolver {
            libs,
            string_table: self.string_table,
        }
    }
}

/// [`FrameResolver`] that pairs a samply-produced Firefox profile with
/// its `.syms.json` sidecar.
pub struct SamplySymsResolver {
    libs: HashMap<String, LibSyms>,
    string_table: Vec<String>,
}

impl SamplySymsResolver {
    fn st(&self, idx: i64) -> &str {
        if idx < 0 {
            return "";
        }
        self.string_table
            .get(idx as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn lookup(&self, debug_name: &str, rva: u64) -> Option<Vec<ResolvedFrame>> {
        let lib = self.libs.get(debug_name)?;
        let table = &lib.symbol_table;
        // Largest entry.rva <= rva.
        let idx = match table.binary_search_by_key(&rva, |e| e.rva) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let sym = &table[idx];
        if rva >= sym.rva + sym.size {
            return None;
        }
        if sym.frames.is_empty() {
            return Some(vec![ResolvedFrame {
                name: self.st(sym.symbol).to_string(),
                file: String::new(),
                line: 0,
                mapping_index: 0,
                address: rva,
            }]);
        }
        // samply stores `frames` outer→inner; pprof's Location.line[]
        // expects leaf-first, so reverse.
        Some(
            sym.frames
                .iter()
                .rev()
                .map(|f| {
                    let mut name = self.st(f.function).to_string();
                    if name.is_empty() {
                        name = self.st(sym.symbol).to_string();
                    }
                    ResolvedFrame {
                        name,
                        file: f.file.map(|i| self.st(i).to_string()).unwrap_or_default(),
                        line: f.line.unwrap_or(0),
                        mapping_index: 0,
                        address: rva,
                    }
                })
                .collect(),
        )
    }
}

impl FrameResolver for SamplySymsResolver {
    fn resolve(
        &self,
        profile: &FirefoxProfile,
        thread: &Thread,
        frame_idx: i64,
    ) -> Vec<ResolvedFrame> {
        let fi = frame_idx as usize;
        let raw_addr = thread.frame_table.address.get(fi).copied().unwrap_or(0);
        let addr = raw_addr.max(0) as u64;
        let func_id = thread.frame_table.func.get(fi).copied().unwrap_or(0) as usize;
        let resource = thread.func_table.resource.get(func_id).copied();
        let lib_idx: usize = match resource {
            Some(r) if r >= 0 => thread
                .resource_table
                .as_ref()
                .and_then(|rt| rt.lib.get(r as usize).copied())
                .filter(|&v| v >= 0)
                .map(|v| v as usize)
                .unwrap_or(0),
            _ => 0,
        };
        let lib = profile.libs.get(lib_idx);
        let resolved = lib
            .and_then(|l| self.lookup(&l.debug_name, addr))
            .map(|frames| {
                frames
                    .into_iter()
                    .map(|mut f| {
                        f.mapping_index = lib_idx;
                        f
                    })
                    .collect::<Vec<_>>()
            });
        if let Some(frames) = resolved {
            return frames;
        }
        let fallback_name = match lib {
            Some(l) => format!("{}+0x{:x}", if l.name.is_empty() { "??" } else { &l.name }, addr),
            None => format!("??+0x{:x}", addr),
        };
        vec![ResolvedFrame {
            name: fallback_name,
            file: String::new(),
            line: 0,
            mapping_index: lib_idx,
            address: addr,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_search_picks_enclosing_symbol() {
        // JSON doesn't allow hex literals; decimals: 0x1000=4096, 0x40=64,
        // 0x2000=8192, 0x80=128.
        let syms_json = r#"{
          "data": [
            {
              "debug_name": "main.exe",
              "symbol_table": [
                { "rva": 8192, "size": 128, "symbol": 1 },
                { "rva": 4096, "size": 64, "symbol": 2, "frames": [
                  { "function": 3, "file": 4, "line": 7 },
                  { "function": 5, "file": 4, "line": 12 }
                ] }
              ]
            }
          ],
          "string_table": ["", "outer_fn", "with_inline", "real_leaf", "src/m.rs", "wrapper"]
        }"#;
        let resolver = SamplySyms::load(syms_json.as_bytes())
            .unwrap()
            .into_resolver();
        let frames = resolver.lookup("main.exe", 0x1010).expect("hit");
        // leaf first: wrapper -> real_leaf (we reversed outer->inner)
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].name, "wrapper");
        assert_eq!(frames[0].line, 12);
        assert_eq!(frames[1].name, "real_leaf");
        assert_eq!(frames[1].line, 7);
        assert_eq!(frames[0].file, "src/m.rs");

        // No inline info -> single ResolvedFrame.
        let frames = resolver.lookup("main.exe", 0x2010).expect("hit");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].name, "outer_fn");
        assert_eq!(frames[0].line, 0);

        // Outside any range -> None.
        assert!(resolver.lookup("main.exe", 0x3000).is_none());
        assert!(resolver.lookup("main.exe", 0x0).is_none());
    }
}
