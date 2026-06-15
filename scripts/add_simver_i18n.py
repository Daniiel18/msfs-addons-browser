# -*- coding: utf-8 -*-
"""Inserta las claves i18n del selector de versión de MSFS (v5.1.0) tras
el ancla `restart.button`, preservando formato y EOL."""
import io

ES = [
    ("simver.title", "¿Qué versión de MSFS usas?"),
    ("simver.subtitle", "Elige tu simulador para ajustar el catálogo y la carpeta Community."),
    ("simver.opt2020", "Microsoft Flight Simulator (2020)."),
    ("simver.opt2024", "Microsoft Flight Simulator 2024."),
    ("simver.choose", "Elegir"),
    ("simver.note", "Podrás cambiarlo luego en Ajustes. La app se recargará al elegir."),
    ("simver.settings.title", "Versión de MSFS"),
    ("simver.settings.description", "Catálogo de SceneryAddons y carpeta Community según tu simulador."),
    ("simver.reload.title", "Reiniciar para aplicar"),
    ("simver.reload.body", "Para usar la versión seleccionada (catálogo y carpeta Community) la app debe recargarse."),
    ("simver.reload.later", "Más tarde"),
    ("simver.reload.now", "Reiniciar ahora"),
]

EN = [
    ("simver.title", "Which MSFS version do you use?"),
    ("simver.subtitle", "Pick your simulator to tune the catalog and Community folder."),
    ("simver.opt2020", "Microsoft Flight Simulator (2020)."),
    ("simver.opt2024", "Microsoft Flight Simulator 2024."),
    ("simver.choose", "Choose"),
    ("simver.note", "You can change it later in Settings. The app reloads on pick."),
    ("simver.settings.title", "MSFS version"),
    ("simver.settings.description", "SceneryAddons catalog and Community folder per your simulator."),
    ("simver.reload.title", "Restart to apply"),
    ("simver.reload.body", "To use the selected version (catalog and Community folder) the app needs to reload."),
    ("simver.reload.later", "Later"),
    ("simver.reload.now", "Restart now"),
]


def insert(path, anchor_key, items):
    with io.open(path, "r", encoding="utf-8", newline="") as f:
        lines = f.readlines()
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
insert(base + r"\es.json", "restart.button", ES)
insert(base + r"\en.json", "restart.button", EN)
