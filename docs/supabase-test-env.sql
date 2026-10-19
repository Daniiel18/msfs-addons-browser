-- (v7.4.2) ENTORNO DE PRUEBAS — tablas propias de la instancia de pruebas.
--
-- La instancia de pruebas (SIMFLEET_SAFE_MODE / --pruebas) NUNCA escribe en las
-- tablas reales: usa `<tabla>_test` y canales Realtime `simfleet-test-vs-*`
-- (ver src/lib/testEnv.ts). Mismo esquema y políticas que las reales.
--
-- Ejecutar UNA vez en el proyecto Supabase de SimFleet:
--   Dashboard → SQL Editor → pegar y Run.
-- Mientras no existan, la instancia de pruebas degrada a solo local (sin errores).

create table if not exists public.vs_results_test (
  channel       text not null,
  identity      text not null,
  name          text,
  callsign      text,
  fpm           integer,
  grade         text,
  registration  text,
  aircraft_type text,
  airline       text,
  updated_at    timestamptz not null default now(),
  primary key (channel, identity)
);

alter table public.vs_results_test enable row level security;

drop policy if exists "vs_results_test anon select" on public.vs_results_test;
drop policy if exists "vs_results_test anon insert" on public.vs_results_test;
drop policy if exists "vs_results_test anon update" on public.vs_results_test;

create policy "vs_results_test anon select" on public.vs_results_test
  for select using (true);
create policy "vs_results_test anon insert" on public.vs_results_test
  for insert with check (true);
create policy "vs_results_test anon update" on public.vs_results_test
  for update using (true) with check (true);

create table if not exists public.pilot_avatars_test (
  identity   text primary key,
  data_url   text not null,
  updated_at timestamptz default now()
);

alter table public.pilot_avatars_test enable row level security;

drop policy if exists "pilot_avatars_test anon all" on public.pilot_avatars_test;
create policy "pilot_avatars_test anon all" on public.pilot_avatars_test
  for all using (true) with check (true);
