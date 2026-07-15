use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use typst::diag::{FileError, FileResult, Warned};
use typst::foundations::{Bytes, Datetime};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::LibraryExt;
use typst::World;
use zhiyuan_core::ResearchReport;

struct PdfWorld {
    library: LazyHash<typst::Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_source: Source,
    sources: HashMap<FileId, Source>,
}

impl World for PdfWorld {
    fn library(&self) -> &LazyHash<typst::Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main_source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.sources
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(PathBuf::new()))
    }

    fn file(&self, _id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(PathBuf::new()))
    }

    fn font(&self, id: usize) -> Option<Font> {
        self.fonts.get(id).cloned()
    }

    fn today(&self, offset: Option<i64>) -> Option<Datetime> {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        let adjusted = secs + offset.unwrap_or(0) * 3600;
        let days = adjusted / 86400;
        let mut y = 1970i32;
        let mut d = days;
        loop {
            let days_in_year = if is_leap(y) { 366 } else { 365 };
            if d < days_in_year {
                break;
            }
            d -= days_in_year;
            y += 1;
        }
        let months = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut m = 0u8;
        for (i, &days_in_m) in months.iter().enumerate() {
            if d < days_in_m {
                m = (i + 1) as u8;
                break;
            }
            d -= days_in_m;
        }
        if m == 0 {
            m = 12;
        }
        let day = (d + 1) as u8;
        Datetime::from_ymd(y, m, day)
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn load_fonts() -> (LazyHash<FontBook>, Vec<Font>) {
    let mut book = FontBook::new();
    let mut fonts = Vec::new();

    for path in find_font_files() {
        if let Ok(data) = std::fs::read(&path) {
            let bytes = Bytes::new(data);
            if let Some(font) = Font::new(bytes, 0) {
                book.push(font.info().clone());
                fonts.push(font);
            }
        }
    }

    (LazyHash::new(book), fonts)
}

fn find_font_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let dirs = [
            format!("{home}/.fonts"),
            format!("{home}/.local/share/fonts"),
            "/usr/share/fonts".into(),
            "/usr/local/share/fonts".into(),
        ];
        for dir in dirs {
            if let Ok(entries) = walk_dir(Path::new(&dir)) {
                for p in entries {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "ttf" | "otf" | "ttc" | "otc") {
                        paths.push(p);
                    }
                }
            }
        }
    }
    paths
}

fn walk_dir(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                result.extend(walk_dir(&path)?);
            } else {
                result.push(path);
            }
        }
    }
    Ok(result)
}

fn markdown_to_typst(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let mut in_code_block = false;

    for line in md.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            result.push_str("```\n");
            continue;
        }

        if in_code_block {
            result.push_str(line);
            result.push('\n');
            continue;
        }

        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            result.push_str("= ");
            result.push_str(rest);
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            result.push_str("== ");
            result.push_str(rest);
        } else if let Some(rest) = trimmed.strip_prefix("### ") {
            result.push_str("=== ");
            result.push_str(rest);
        } else if let Some(rest) = trimmed.strip_prefix("#### ") {
            result.push_str("==== ");
            result.push_str(rest);
        } else {
            result.push_str(&line.replace("**", "*"));
        }
        result.push('\n');
    }

    result
}

fn generate_typst_source(report: &ResearchReport) -> String {
    let mut typ = String::new();

    typ.push_str("#set page(\n");
    typ.push_str("  margin: (x: 2.5cm, y: 2cm),\n");
    typ.push_str("  numbering: \"1\",\n");
    typ.push_str(")\n\n");

    typ.push_str("#set text(font: \"Noto Sans CJK SC\", size: 11pt)\n\n");
    typ.push_str("#show heading.where(level: 1): it => [\n");
    typ.push_str("  #v(1cm)\n");
    typ.push_str("  #align(center, text(size: 18pt, weight: \"bold\", it.body))\n");
    typ.push_str("  #v(0.3cm)\n");
    typ.push_str("  #align(center, text(size: 9pt, fill: gray))[\n");
    typ.push_str(&format!(
        "    生成于 {}\n",
        report.generated_at.format("%Y-%m-%d %H:%M UTC")
    ));
    typ.push_str("  ]\n");
    typ.push_str("  #v(0.5cm)\n");
    typ.push_str("]\n\n");

    typ.push_str("#show heading.where(level: 2): it => [\n");
    typ.push_str("  #v(0.5cm)\n");
    typ.push_str("  #text(size: 14pt, weight: \"bold\", it.body)\n");
    typ.push_str("  #v(0.2cm)\n");
    typ.push_str("]\n\n");

    typ.push_str("#show heading.where(level: 3): it => [\n");
    typ.push_str("  #text(size: 12pt, weight: \"bold\", it.body)\n");
    typ.push_str("]\n\n");

    typ.push_str("#set par(justify: true, leading: 0.65em)\n\n");

    typ.push_str("= ");
    typ.push_str(&report.title);
    typ.push_str("\n\n");

    for section in &report.sections {
        if section.content.is_empty() {
            continue;
        }
        let converted = markdown_to_typst(&section.content);
        typ.push_str(&converted);
        typ.push('\n');
    }

    let citations: Vec<&str> = report
        .citation_graph
        .sources
        .iter()
        .map(|s| s.url.as_str())
        .collect();

    if !citations.is_empty() {
        typ.push_str("= 参考文献\n\n");
        for (i, url) in citations.iter().enumerate() {
            typ.push_str(&format!("{}. {}\n", i + 1, url));
        }
        typ.push('\n');
    }

    let q = &report.quality_score;
    typ.push_str("---\n\n");
    typ.push_str(&format!(
        "质量评分：{}（覆盖率：{:.1}%，可靠性：{:.1}%，深度：{:.1}%）\n",
        q.overall,
        q.coverage * 100.0,
        q.reliability * 100.0,
        q.depth * 100.0,
    ));

    typ
}

pub fn compile_report(report: &ResearchReport, output_path: &Path) -> anyhow::Result<()> {
    let (book, fonts) = load_fonts();

    let main_id = FileId::new(None, VirtualPath::new(Path::new("/main.typ")));
    let typst_source = generate_typst_source(report);
    let main_source = Source::new(main_id, typst_source);

    let mut sources = HashMap::new();
    sources.insert(main_id, main_source.clone());

    let world = PdfWorld {
        library: LazyHash::new(typst::Library::default()),
        book,
        fonts,
        main_source,
        sources,
    };

    let Warned { output, warnings } = typst::compile::<PagedDocument>(&world);

    let document = output.map_err(|diags| {
        anyhow::anyhow!(
            "Typst compilation failed: {}",
            diags
                .iter()
                .map(|d| format!("{}", d.message))
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;

    for warning in &warnings {
        tracing::warn!("Typst: {}", warning.message);
    }

    let pdf_options = typst_pdf::PdfOptions {
        ident: typst::foundations::Smart::Auto,
        timestamp: None,
        page_ranges: None,
        standards: typst_pdf::PdfStandards::default(),
        tagged: false,
    };

    let pdf_bytes = typst_pdf::pdf(&document, &pdf_options)
        .map_err(|e| anyhow::anyhow!("PDF export failed: {:?}", e))?;

    std::fs::write(output_path, &pdf_bytes)?;
    tracing::info!("PDF written to {}", output_path.display());

    Ok(())
}
