/** A file name that keeps its extension: when it is too long, the stem gives way. */
export function FileName({ value }: { value: string }) {
  const dot = value.lastIndexOf('.')
  const split = dot > 0 ? dot : value.length
  return (
    <span class="file-name" title={value}>
      <span>{value.slice(0, split)}</span>
      <span>{value.slice(split)}</span>
    </span>
  )
}
