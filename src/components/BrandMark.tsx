/**
 * (19/10) Logo de SimFleet — monograma "S": la S dibujada como una ruta de
 * vuelo en el teal de la interfaz, con su origen y el avión al final.
 * Mismo dibujo que el icono de la app (`src-tauri/icons/source-icon.svg`).
 */
export function BrandMark({
  className,
  title,
}: {
  className?: string;
  /** Texto accesible; sin él el logo es decorativo. */
  title?: string;
}) {
  return (
    <svg
      viewBox="0 0 512 512"
      fill="none"
      className={className}
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
    >
      {title && <title>{title}</title>}
      <rect width="512" height="512" rx="112" fill="#0B1A1C" />
      <path
        d="M150 372 C150 300 360 312 360 238 C360 170 160 196 160 134"
        stroke="#35CDB2"
        strokeWidth="30"
        strokeLinecap="round"
      />
      <circle cx="150" cy="380" r="26" fill="#35CDB2" />
      <g transform="translate(176 118) rotate(-62) scale(5.6)">
        <path
          d="M12 0C12-1.5 10-2 8-2L2-2-4-11H-7L-3-2H-9L-12-6H-14L-12.5 0-14 6H-12L-9 2H-3L-7 11H-4L2 2H8C10 2 12 1.5 12 0Z"
          fill="#FFFFFF"
        />
      </g>
    </svg>
  );
}
