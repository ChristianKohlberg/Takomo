// The takomo mark (a waveform), as JSX.
//
// The pages injected this through `innerHTML` because a string was the cheapest
// thing to inline. It is a component now, which is both safer and the reason the
// `innerHTML` ban can be absolute rather than "except for the logo".
export function Logo({ size = 22, className }: { size?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="M6 11a6 6 0 0 1 12 0" />
      <circle cx="9.7" cy="9.6" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="14.3" cy="9.6" r="0.9" fill="currentColor" stroke="none" />
      <path d="M7 11.2c0 2.8-1.4 3.8-1.4 5.8" />
      <path d="M10 11.6c0 2.9-.5 4.3 0 6.2" />
      <path d="M14 11.6c0 2.9.5 4.3 0 6.2" />
      <path d="M17 11.2c0 2.8 1.4 3.8 1.4 5.8" />
    </svg>
  )
}
