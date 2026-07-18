#let project(
  title: "",
  subtitle: none,
  heading-numbering: "1.1",
  body
) = {
  set document(author: "致远", title: title)

  set page(
    numbering: "1",
    number-align: center,
    foreground: context {
      if counter(page).get().first() > 1 {
        place(top + right, dx: -15pt, dy: 15pt, image("icon.svg", width: 50pt))
      }
    },
    footer: context {
      let page-number = counter(page).get().at(0)
      if page-number > 1 {
        line(length: 100%, stroke: 0.5pt)
        v(-2pt)
        text(size: 12pt, weight: "regular")[
          致远
          #h(1fr)
          #page-number
        ]
      }
    }
  )

  set text(size: 13pt)
  set heading(numbering: heading-numbering)

  show heading: it => {
    if it.level == 1 and it.numbering != none {
      v(40pt)
      text(size: 30pt)[#counter(heading).display() #linebreak() #it.body]
      v(60pt)
    } else {
      v(5pt)
      [#it]
      v(12pt)
    }
  }

  // 封面内容
  v(1fr)
  align(center + horizon)[
    #line(length: 100%, stroke: 0.5pt)
    #text(size: 20pt, weight: "bold")[#title]
    #line(length: 100%, stroke: 0.5pt)
  ]

  v(1fr)
  // 底部：框架声明 + 大图标
  align(center + bottom)[
    #image("icon.svg", width: 80%) // 按需调整大小
    #text(size: 10pt, style: "italic")[致远本地深度研究生成，内容由AI生成，请仔细甄别]
  ]

  pagebreak()
  outline(title: "目录", depth: 2, indent: auto)
  body
}
