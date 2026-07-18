use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use base64::Engine;
use typst::diag::{FileError, FileResult, Warned};
use typst::foundations::{Bytes, Datetime};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::LibraryExt;
use typst::World;
use zhiyuan_core::{ResearchReport, SourceNode};

pub struct ParaSpan {
    pub section_idx: usize,
    pub content_start: usize,
    pub content_end: usize,
    pub source_line_start: usize,
    pub source_line_end: usize,
}

pub struct SourceMap {
    pub spans: Vec<ParaSpan>,
}

impl SourceMap {
    pub fn span_at_line(&self, line: usize) -> Option<&ParaSpan> {
        self.spans
            .iter()
            .find(|s| line >= s.source_line_start && line < s.source_line_end)
    }
}

pub struct SourceError {
    pub line: usize,
    pub message: String,
}

struct PdfWorld {
    library: LazyHash<typst::Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_source: Source,
    sources: HashMap<FileId, Source>,
    bib_bytes: Option<Bytes>,
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

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id.vpath().as_rooted_path().ends_with("works.bib") {
            if let Some(ref bib) = self.bib_bytes {
                return Ok(bib.clone());
            }
        }
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
        let months = [
            31,
            if is_leap(y) { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
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

fn load_fonts(font_paths: &[String]) -> (LazyHash<FontBook>, Vec<Font>) {
    let mut book = FontBook::new();
    let mut fonts = Vec::new();

    for path in font_paths {
        let Ok(data) = std::fs::read(path) else {
            tracing::warn!("字体文件无法读取: {path}");
            continue;
        };
        let bytes = Bytes::new(data);
        for i in 0..8 {
            match Font::new(bytes.clone(), i) {
                Some(font) => {
                    book.push(font.info().clone());
                    fonts.push(font);
                }
                None if i == 0 => {
                    tracing::warn!("字体加载失败: {path}");
                    break;
                }
                None => break,
            }
        }
    }

    if fonts.is_empty() {
        tracing::warn!("未加载任何字体，PDF 输出可能不正常");
    }

    (LazyHash::new(book), fonts)
}

pub fn bib_key(url: &str) -> String {
    let url = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let domain = url.split('/').next().unwrap_or("unknown");
    let prefix = domain
        .trim_start_matches("www.")
        .split('.')
        .next()
        .unwrap_or("x");
    let path = url.trim_start_matches(domain).trim_matches('/');
    let slug: String = path
        .split('/')
        .last()
        .unwrap_or("")
        .trim_end_matches(".pdf")
        .trim_end_matches(".html")
        .trim_end_matches(".htm")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let slug = slug.trim_matches('-').to_lowercase();
    if slug.is_empty() || slug.len() < 3 {
        prefix.to_string()
    } else {
        format!("{}_{}", prefix, slug)
    }
}

pub fn generate_bibliography(sources: &[SourceNode]) -> String {
    let mut bib = String::new();
    for s in sources {
        let key = bib_key(&s.url);
        let title = s.title.replace('"', "'");
        bib.push_str(&format!(
            "@misc{{{key},
  title = {{{title}}},
  url = {{{}}}
}}\n\n",
            s.url
        ));
    }
    bib
}

fn quote_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn generate_typst_source(report: &ResearchReport) -> (String, SourceMap) {
    let mut typ = String::new();
    let mut spans = Vec::new();

    fn cur_line(s: &str) -> usize {
        s.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1
    }

    // preamble
    let preamble = include_str!("../template/lib.typ");
    let icon_data = base64::engine::general_purpose::STANDARD.encode(include_bytes!("../template/icon.svg"));
    let preamble = preamble.replace(
        "image(\"icon.svg\"",
        &format!("image(\"data:image/svg+xml;base64,{}\"", icon_data),
    );

    typ.push_str(&preamble);
    typ.push_str(&format!("\n#show: project.with(title: {})\n\n", quote_string(&report.title)));

    // sections
    for (si, section) in report.sections.iter().enumerate() {
        if section.content.is_empty() {
            continue;
        }

        // split by blank lines → paragraphs
        let mut content_offset = 0usize;
        for para in section.content.split("\n\n") {
            let para = para.trim();
            if para.is_empty() {
                content_offset += 2;
                continue;
            }

            let para_start = cur_line(&typ);
            typ.push_str(para);
            typ.push_str("\n\n");
            let para_end = cur_line(&typ);

            spans.push(ParaSpan {
                section_idx: si,
                content_start: content_offset,
                content_end: content_offset + para.len(),
                source_line_start: para_start,
                source_line_end: para_end,
            });

            content_offset += para.len() + 2;
        }
    }

    // citations
    if !report.citation_graph.sources.is_empty() {
        typ.push_str("= 参考文献\n\n#bibliography(\"works.bib\", title: none)\n");
    }

    (typ, SourceMap { spans })
}

pub fn compile_source_detailed(
    source: &str,
    font_paths: &[String],
    bib_source: Option<&str>,
) -> std::result::Result<Vec<u8>, Vec<SourceError>> {
    let main_id = FileId::new(None, VirtualPath::new(Path::new("/main.typ")));
    let main_source = Source::new(main_id, source.to_string());
    let mut sources = HashMap::new();
    sources.insert(main_id, main_source.clone());

    let (book, fonts) = load_fonts(font_paths);
    let bib_bytes = bib_source.map(|s| Bytes::new(s.as_bytes().to_vec()));
    let world = PdfWorld {
        library: LazyHash::new(typst::Library::default()),
        book,
        fonts,
        main_source,
        sources,
        bib_bytes,
    };

    let Warned { output, warnings } = typst::compile::<PagedDocument>(&world);

    for warning in &warnings {
        tracing::warn!("Typst: {}", warning.message);
    }

    match output {
        Ok(document) => {
            let pdf_options = typst_pdf::PdfOptions {
                ident: typst::foundations::Smart::Auto,
                timestamp: None,
                page_ranges: None,
                standards: typst_pdf::PdfStandards::default(),
                tagged: false,
            };
            let pdf_bytes = typst_pdf::pdf(&document, &pdf_options).map_err(|e| {
                vec![SourceError {
                    line: 0,
                    message: format!("PDF export failed: {:?}", e),
                }]
            })?;
            Ok(pdf_bytes)
        }
        Err(diags) => {
            let src = source.as_bytes();
            let errors: Vec<SourceError> = diags
                .iter()
                .map(|d| {
                    let line = world
                        .main_source
                        .range(d.span)
                        .and_then(|r| {
                            if r.start < src.len() {
                                Some(src[..r.start].iter().filter(|&&c| c == b'\n').count() + 1)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    SourceError {
                        line,
                        message: d.message.to_string(),
                    }
                })
                .collect();
            Err(errors)
        }
    }
}
