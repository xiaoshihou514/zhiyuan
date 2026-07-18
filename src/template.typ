#set page(
  margin: (x: 2.5cm, y: 2cm),
  numbering: "1",
)

#set text(font: "Noto Sans CJK SC", size: 11pt)

#show heading.where(level: 1): it => [
  #v(1cm)
  #align(center, text(size: 18pt, weight: "bold", it.body))
  #v(0.3cm)
]

#show heading.where(level: 2): it => [
  #v(0.5cm)
  #text(size: 14pt, weight: "bold", it.body)
  #v(0.2cm)
]

#show heading.where(level: 3): it => [
  #text(size: 12pt, weight: "bold", it.body)
]

#set par(justify: true, leading: 0.65em)
