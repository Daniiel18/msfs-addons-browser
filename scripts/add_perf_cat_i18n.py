# -*- coding: utf-8 -*-
"""Inserta las claves i18n por categoría del modal de FPS (v5.0.0) tras
el ancla `perf.scan_done`, preservando formato y EOL."""
import io

ES = [
    ("perf.cat.passengers.label", "Pasajeros 3D"),
    ("perf.cat.passengers.desc", "Quita los pasajeros 3D de terminales y puertas de embarque."),
    ("perf.cat.static_aircraft.label", "Aeronaves estáticas"),
    ("perf.cat.static_aircraft.desc", "Quita las aeronaves estáticas (AI parqueadas decorativas)."),
    ("perf.cat.vdgs.label", "Cajas VDGS"),
    ("perf.cat.vdgs.desc", "Quita las cajas VDGS de Aerosoft (úsalo si manejas VDGS con GSX)."),
    ("perf.cat.gse.label", "Equipo de tierra (GSE)"),
    ("perf.cat.gse.desc", "Quita el equipo de tierra estático (tractores, cintas, escaleras, GPU)."),
    ("perf.cat.static_cars.label", "Autos estáticos"),
    ("perf.cat.static_cars.desc", "Quita los autos estáticos del aparcamiento y viales del aeropuerto."),
    ("perf.cat.clutter.label", "Clutter / objetos extra"),
    ("perf.cat.clutter.desc", "Quita el clutter no esencial (modelos, farolas, vallas)."),
    ("perf.cat.road_traffic.label", "Tráfico de carretera"),
    ("perf.cat.road_traffic.desc", "Quita el tráfico de carretera alrededor del aeropuerto."),
    ("perf.cat.service_traffic.label", "Tráfico de servicio"),
    ("perf.cat.service_traffic.desc", "Quita los vehículos de servicio en movimiento por la plataforma."),
    ("perf.cat.ramp_personnel.label", "Personal de rampa"),
    ("perf.cat.ramp_personnel.desc", "Quita el personal de rampa estático (operarios 3D en plataforma)."),
    ("perf.cat.animated.label", "Agentes animados"),
    ("perf.cat.animated.desc", "Quita los agentes y vehículos animados (los que más cuestan FPS)."),
    ("perf.cat.other.label", "Objetos opcionales"),
    ("perf.cat.other.desc", "Objetos opcionales del escenario que puedes desactivar para ganar FPS."),
]

EN = [
    ("perf.cat.passengers.label", "3D Passengers"),
    ("perf.cat.passengers.desc", "Removes 3D passengers from terminals and boarding gates."),
    ("perf.cat.static_aircraft.label", "Static aircraft"),
    ("perf.cat.static_aircraft.desc", "Removes decorative static (parked AI) aircraft."),
    ("perf.cat.vdgs.label", "VDGS boxes"),
    ("perf.cat.vdgs.desc", "Removes Aerosoft VDGS boxes (use this if you run VDGS with GSX)."),
    ("perf.cat.gse.label", "Ground equipment (GSE)"),
    ("perf.cat.gse.desc", "Removes static ground service equipment (tugs, belts, stairs, GPU)."),
    ("perf.cat.static_cars.label", "Static cars"),
    ("perf.cat.static_cars.desc", "Removes static cars from parking and airport roads."),
    ("perf.cat.clutter.label", "Clutter / extra objects"),
    ("perf.cat.clutter.desc", "Removes non-essential clutter (models, streetlights, fences)."),
    ("perf.cat.road_traffic.label", "Road traffic"),
    ("perf.cat.road_traffic.desc", "Removes road traffic around the airport."),
    ("perf.cat.service_traffic.label", "Service traffic"),
    ("perf.cat.service_traffic.desc", "Removes moving service vehicles on the apron."),
    ("perf.cat.ramp_personnel.label", "Ramp personnel"),
    ("perf.cat.ramp_personnel.desc", "Removes static ramp personnel (3D ground crew)."),
    ("perf.cat.animated.label", "Animated agents"),
    ("perf.cat.animated.desc", "Removes animated agents and vehicles (the heaviest on FPS)."),
    ("perf.cat.other.label", "Optional objects"),
    ("perf.cat.other.desc", "Optional scenery objects you can disable to gain FPS."),
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
insert(base + r"\es.json", "perf.scan_done", ES)
insert(base + r"\en.json", "perf.scan_done", EN)
