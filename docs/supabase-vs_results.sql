-- (v6.2.3) Crew VS ASÍNCRONO — tabla de resultados persistidos por vuelo+piloto.
--
-- El realtime (presence/broadcast) sólo sirve si Daniel y Héctor están conectados
-- A LA VEZ. Esta tabla guarda el resultado de cada piloto (FPM, avión real) por
-- vuelo (channel = fecha-origen-destino), para que el resultado del rival aparezca
-- AUNQUE no coincidan online: uno vuela, sale del sim, y el otro lo ve después.
--
-- Ejecutar UNA vez en el proyecto Supabase de SimFleet:
--   Dashboard → SQL Editor → pegar y Run.
-- Mientras la tabla no exista, el Crew VS sigue funcionando con realtime +
-- localStorage (el código falla en silencio); esta tabla añade el modo asíncrono.

create table if not exists public.vs_results (
  channel       text not null,           -- simfleet-vs-<fecha>-<origen>-<destino>
  identity      text not null,           -- "daniel" | "hector"
  name          text,
  callsign      text,
  fpm           integer,                 -- NULL hasta aterrizar
  grade         text,                    -- "butter" | "acceptable" | "hard"
  registration  text,                    -- matrícula real del sim
  aircraft_type text,
  airline       text,
  updated_at    timestamptz not null default now(),
  primary key (channel, identity)
);

alter table public.vs_results enable row level security;

-- App privada de 2 personas con la anon key embebida: permitimos lectura y
-- escritura anónimas. (Si más adelante se quiere endurecer, atar por identity.)
drop policy if exists "vs_results anon select" on public.vs_results;
drop policy if exists "vs_results anon insert" on public.vs_results;
drop policy if exists "vs_results anon update" on public.vs_results;

create policy "vs_results anon select" on public.vs_results
  for select using (true);
create policy "vs_results anon insert" on public.vs_results
  for insert with check (true);
create policy "vs_results anon update" on public.vs_results
  for update using (true) with check (true);
