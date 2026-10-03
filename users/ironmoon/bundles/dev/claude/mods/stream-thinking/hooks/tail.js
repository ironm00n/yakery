function wrap(line, width) {
  const rows = []
  let row = ''
  for (const word of line.split(' ')) {
    const joined = row ? `${row} ${word}` : word
    if (joined.length <= width) {
      row = joined
      continue
    }
    if (row) rows.push(row)
    row = word
    while (row.length > width) {
      rows.push(row.slice(0, width))
      row = row.slice(width)
    }
  }
  return [...rows, row]
}

export const tail = (text, width, rows) =>
  text
    .split('\n')
    .flatMap((line) => wrap(line, width))
    .slice(-rows)
    .join('\n')
