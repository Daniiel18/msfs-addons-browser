# -*- coding: utf-8 -*-
"""Inserta las claves i18n del motor de rendimiento (v5.0.0) tras el
ancla `find.close`, preservando formato y EOL del archivo."""
import io

ES = [
    ("perf.open_button", "Optimizar FPS"),
    ("perf.title", "Optimización de FPS"),
    ("perf.help", "Cada interruptor controla la PRESENCIA de objetos pesados opcionales. Apágalo para quitarlos del escenario (renombra el .bgl a .bgl.off) y ganar FPS; enciéndelo para restaurarlos. Cierra MSFS o vuelve al menú principal antes de cambiarlos."),
    ("perf.load_error", "No se pudo leer la configuración de rendimiento"),
    ("perf.toggle_error", "No se pudo aplicar el cambio"),
    ("perf.restored", "{label} restaurado"),
    ("perf.disabled", "{label} desactivado · +FPS"),
    ("perf.renamed", "{n} archivo(s) renombrado(s)"),
    ("perf.no_source", "No se encontró la ficha en SceneryAddons"),
    ("perf.enriched", "Opciones actualizadas desde SceneryAddons"),
    ("perf.enrich_error", "No se pudo actualizar desde SceneryAddons"),
    ("perf.enrich", "Actualizar desde SceneryAddons"),
    ("perf.enrich_hint", "Baja la nota “Optional Configuration” del desarrollador para mejorar etiquetas y estimados de FPS."),
    ("perf.empty.title", "Sin objetos opcionales"),
    ("perf.empty.body", "Este escenario no trae objetos pesados desactivables (autos, pasajeros, GSE…), o no pudimos detectarlos. Prueba “Actualizar desde SceneryAddons”."),
    ("perf.one_file", "1 archivo"),
    ("perf.n_files", "{n} archivos"),
    ("perf.off_now", "desactivado ahora"),
    ("perf.on", "Activo"),
    ("perf.off", "Apagado"),
    ("perf.footer_saving", "{n} optimización(es) activa(s)"),
    ("perf.footer_full", "Escenario completo"),
    ("tour.perf.title", "Optimizador de FPS (nuevo)"),
    ("tour.perf.body", "En el Mapa, abre cualquier aeropuerto instalado y pulsa “Optimizar FPS”. Verás interruptores para quitar autos estáticos, pasajeros 3D, GSE o agentes animados — cada uno con su ahorro estimado. La app renombra los .bgl por ti, de forma segura."),
    ("tour.linkmap.title", "Link Map y aeropuertos por región"),
    ("tour.linkmap.body", "Cambia a “Link Map” para ver tus addons como un grafo: aviones y sus liveries enlazados, y tus aeropuertos agrupados por continente a un lado con su badge de región. Usa “Auto-organizar” para acomodarlo todo de un clic."),
    ("tour.addons_batch.title", "Acciones rápidas"),
    ("tour.addons_batch.body", "Enciende o apaga todos tus addons de golpe, y usa “Refresh” para re-escanear Community (también regenera las opciones de FPS de tus aeropuertos)."),
]

EN = [
    ("perf.open_button", "Optimize FPS"),
    ("perf.title", "FPS Optimization"),
    ("perf.help", "Each switch controls the PRESENCE of optional heavy objects. Turn it off to remove them from the scenery (renames the .bgl to .bgl.off) and gain FPS; turn it on to restore them. Close MSFS or return to the main menu before changing them."),
    ("perf.load_error", "Couldn't read the performance config"),
    ("perf.toggle_error", "Couldn't apply the change"),
    ("perf.restored", "{label} restored"),
    ("perf.disabled", "{label} disabled · +FPS"),
    ("perf.renamed", "{n} file(s) renamed"),
    ("perf.no_source", "No SceneryAddons page found"),
    ("perf.enriched", "Options updated from SceneryAddons"),
    ("perf.enrich_error", "Couldn't update from SceneryAddons"),
    ("perf.enrich", "Update from SceneryAddons"),
    ("perf.enrich_hint", "Fetch the developer's “Optional Configuration” note to improve labels and FPS estimates."),
    ("perf.empty.title", "No optional objects"),
    ("perf.empty.body", "This scenery has no removable heavy objects (cars, passengers, GSE…), or we couldn't detect them. Try “Update from SceneryAddons”."),
    ("perf.one_file", "1 file"),
    ("perf.n_files", "{n} files"),
    ("perf.off_now", "off now"),
    ("perf.on", "On"),
    ("perf.off", "Off"),
    ("perf.footer_saving", "{n} optimization(s) active"),
    ("perf.footer_full", "Full scenery"),
    ("tour.perf.title", "FPS Optimizer (new)"),
    ("tour.perf.body", "In the Map, open any installed airport and hit “Optimize FPS”. You'll get switches to remove static cars, 3D passengers, GSE or animated agents — each with its estimated saving. The app renames the .bgl files for you, safely."),
    ("tour.linkmap.title", "Link Map & airports by region"),
    ("tour.linkmap.body", "Switch to “Link Map” to see your addons as a graph: aircraft linked to their liveries, and your airports grouped by continent on one side with their region badge. Use “Auto-arrange” to lay it all out in one click."),
    ("tour.addons_batch.title", "Quick actions"),
    ("tour.addons_batch.body", "Enable or disable all your addons at once, and use “Refresh” to re-scan Community (it also regenerates your airports' FPS options)."),
]


def insert(path, anchor_key, items):
    with io.open(path, "r", encoding="utf-8", newline="") as f:
        lines = f.readlines()
    # Evita duplicar si ya existen (re-ejecución idempotente).
    existing = "".join(lines)
    items = [(k, v) for (k, v) in items if ('"%s":' % k) not in existing]
    if not items:
        print("nada que insertar en", path)
        return
    out = []
    inserted = False
    for line in lines:
        out.append(line)
        if not inserted and ('"%s":' % anchor_key) in line:
            nl = "\r\n" if line.endswith("\r\n") else "\n"
            for k, v in items:
                out.append('  "%s": "%s",%s' % (k, v, nl))
            inserted = True
    if not inserted:
        raise SystemExit("ancla '%s' no encontrada en %s" % (anchor_key, path))
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.writelines(out)
    print("insertadas", len(items), "claves en", path)


base = r"C:\Users\n0xful\source\repos\msfs-addons-browser\src\lib\i18n"
insert(base + r"\es.json", "find.close", ES)
insert(base + r"\en.json", "find.close", EN)
