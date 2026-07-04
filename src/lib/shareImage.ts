import { api } from "./tauri";

/**
 * (v6.2.30 / R7) Utilidades para exportar una tarjeta SVG como imagen.
 *
 * Rasterizamos el SVG (autocontenido: vector + texto, sin imágenes
 * externas para evitar canvas "tainted") a PNG en un canvas y luego:
 *   · lo copiamos al portapapeles (compartir pegando en Discord/WhatsApp)
 *   · o lo guardamos a disco vía el comando backend `save_binary_file`.
 */

export async function svgToPngBlob(
  svg: string,
  width: number,
  height: number,
  scale = 2,
): Promise<Blob> {
  const svgBlob = new Blob([svg], { type: "image/svg+xml;charset=utf-8" });
  const url = URL.createObjectURL(svgBlob);
  try {
    const img = new Image();
    img.width = width;
    img.height = height;
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("no se pudo rasterizar el SVG"));
      img.src = url;
    });
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(width * scale);
    canvas.height = Math.round(height * scale);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d no disponible");
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0, width, height);
    return await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob(
        (b) => (b ? resolve(b) : reject(new Error("toBlob devolvió null"))),
        "image/png",
      ),
    );
  } finally {
    URL.revokeObjectURL(url);
  }
}

export async function copyImageToClipboard(blob: Blob): Promise<boolean> {
  try {
    // ClipboardItem puede no existir en runtimes viejos.
    const CI = (window as unknown as { ClipboardItem?: typeof ClipboardItem })
      .ClipboardItem;
    if (!CI || !navigator.clipboard?.write) return false;
    await navigator.clipboard.write([new CI({ "image/png": blob })]);
    return true;
  } catch {
    return false;
  }
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(String(fr.result));
    fr.onerror = () => reject(new Error("no se pudo leer el blob"));
    fr.readAsDataURL(blob);
  });
}

export type SaveResult = "saved" | "cancelled" | "error";

export async function savePngToDisk(
  blob: Blob,
  defaultName: string,
): Promise<SaveResult> {
  try {
    const path = await api.pickSavePath(defaultName, [
      { name: "PNG", extensions: ["png"] },
    ]);
    if (!path) return "cancelled";
    const dataUrl = await blobToDataUrl(blob);
    await api.saveBinaryFile(path, dataUrl);
    return "saved";
  } catch {
    return "error";
  }
}
