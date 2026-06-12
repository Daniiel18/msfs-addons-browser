import { useEffect, useState } from "react";
import { api } from "./tauri";

/**
 * (v4.25.0) Hook compartido de thumbnails de paquetes Community.
 *
 * Antes vivía duplicado dentro de AddonsView; ahora lo consumen las
 * cards del grid, los nodos del Link Map y el popover del mapa. El
 * cache es a nivel de módulo (sobrevive a remounts) y guarda `null`
 * para los paquetes sin imagen — así no re-intentamos en cada render.
 */
const thumbnailCache = new Map<string, string | null>();

export function useThumbnail(
  folderName: string,
  skip: boolean = false,
): string | null {
  const cached = thumbnailCache.get(folderName);
  const [src, setSrc] = useState<string | null>(cached ?? null);
  useEffect(() => {
    // Si el caller dice "skip" (título de placeholder/test), ni
    // siquiera intentamos cargar el thumbnail. Cacheamos null para
    // evitar re-intentos.
    if (skip) {
      thumbnailCache.set(folderName, null);
      setSrc(null);
      return;
    }
    if (thumbnailCache.has(folderName)) {
      setSrc(thumbnailCache.get(folderName) ?? null);
      return;
    }
    let cancelled = false;
    api
      .packageThumbnail(folderName)
      .then((dataUrl) => {
        if (cancelled) return;
        // Heurística anti-placeholder: si el data URL es muy chico
        // (<3 KB de base64 ≈ <2 KB de imagen real), probablemente
        // es un PNG genérico "PLACEHOLDER" que el dev dejó como
        // marcador. Cacheamos null y renderizamos el icono de
        // categoría en su lugar.
        const looksTiny = dataUrl !== null && dataUrl.length < 3000;
        const finalUrl = looksTiny ? null : dataUrl;
        thumbnailCache.set(folderName, finalUrl);
        setSrc(finalUrl);
      })
      .catch(() => {
        if (cancelled) return;
        thumbnailCache.set(folderName, null);
        setSrc(null);
      });
    return () => {
      cancelled = true;
    };
  }, [folderName, skip]);
  return src;
}
