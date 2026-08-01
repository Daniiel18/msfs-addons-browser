import { useEffect, useRef, useState } from "react";
import { Camera, ChevronDown, Trophy } from "lucide-react";
import type { PilotProfile } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import {
  loadAvatar,
  saveAvatar,
  subscribeAvatar,
} from "../lib/avatarStore";
import { AchievementsModal } from "./AchievementsModal";

/**
 * (v6.1 #30) Insignia del piloto — vive en el header GLOBAL (a la derecha del
 * botón Downloads) para que el nivel/avatar salga en TODAS las pantallas, no
 * solo en el Hangar. Despliega un menú con foto editable + nivel/XP. El avatar
 * persiste local + nube (solo la foto, vía Supabase — ver avatarStore).
 */
export function PilotBadge() {
  const [profile, setProfile] = useState<PilotProfile | null>(null);

  useEffect(() => {
    api.pilotProfile().then(setProfile).catch(() => {});
  }, []);

  if (!profile) return null;
  return <PilotMenu profile={profile} />;
}

function PilotMenu({ profile }: { profile: PilotProfile }) {
  const [open, setOpen] = useState(false);
  // (v7) Panel de Logros abierto desde el perfil.
  const [achOpen, setAchOpen] = useState(false);
  const [avatar, setAvatar] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // (v6.1 #30) Avatar local + nube: carga inicial + suscripción a cambios
  // (p.ej. la foto bajó desde otra PC vía Supabase).
  useEffect(() => {
    loadAvatar(profile.identity).then(setAvatar).catch(() => {});
    return subscribeAvatar(profile.identity, setAvatar);
  }, [profile.identity]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const onPick = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      setAvatar(url);
      // Persiste local + sube a la nube (solo la foto).
      void saveAvatar(profile.identity, url);
    };
    reader.readAsDataURL(file);
  };

  const pct =
    profile.xpForNext > 0
      ? Math.min(100, Math.round((profile.xpInLevel / profile.xpForNext) * 100))
      : 0;

  return (
    <div className="relative" ref={menuRef}>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-2 rounded-xl border border-slate-800 bg-slate-900/60 py-1 pl-1 pr-2 hover:border-brand-500/40"
      >
        <Avatar avatar={avatar} identity={profile.identity} size={28} />
        <div className="hidden text-left sm:block">
          <div className="text-xs font-semibold leading-tight text-slate-100">
            {profile.name}
          </div>
          <div className="text-[10px] leading-tight text-brand-300">
            {t("hangar.profile.level", { n: String(profile.level) })}
          </div>
        </div>
        <ChevronDown className="h-3.5 w-3.5 text-slate-500" />
      </button>

      {open && (
        <div className="absolute right-0 z-50 mt-2 w-64 rounded-2xl border border-slate-800 bg-slate-950 p-4 shadow-2xl ring-1 ring-slate-800">
          <div className="flex flex-col items-center">
            <div className="relative">
              <Avatar avatar={avatar} identity={profile.identity} size={64} />
              <button
                onClick={() => fileRef.current?.click()}
                title={t("hangar.profile.change_photo")}
                className="absolute -bottom-1 -right-1 rounded-full border border-slate-700 bg-slate-800 p-1.5 text-slate-300 hover:bg-slate-700"
              >
                <Camera className="h-3 w-3" />
              </button>
              <input
                ref={fileRef}
                type="file"
                accept="image/*"
                className="hidden"
                onChange={onPick}
              />
            </div>
            <div className="mt-2 text-sm font-semibold text-slate-100">
              {profile.name}
            </div>
            <button
              onClick={() => fileRef.current?.click()}
              className="text-[11px] text-brand-300 hover:text-brand-200"
            >
              {t("hangar.profile.change_photo")}
            </button>
          </div>

          {/* Nivel + XP */}
          <div className="mt-4 rounded-xl border border-slate-800 bg-slate-900/50 p-3">
            <div className="flex items-center justify-between text-xs">
              <span className="font-semibold text-slate-200">
                {t("hangar.profile.level", { n: String(profile.level) })}
              </span>
              <span className="text-slate-500">
                {profile.xpInLevel.toLocaleString()} /{" "}
                {profile.xpForNext.toLocaleString()} XP
              </span>
            </div>
            <div className="mt-2 h-2 overflow-hidden rounded-full bg-slate-800">
              <div
                className="h-full bg-gradient-to-r from-brand-500 to-brand-400"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="mt-1.5 text-[10px] text-slate-500">
              {t("hangar.profile.to_next", {
                n: Math.max(
                  0,
                  profile.xpForNext - profile.xpInLevel,
                ).toLocaleString(),
              })}
            </div>
          </div>

          <div className="mt-3 grid grid-cols-2 gap-2 text-center">
            <div className="rounded-lg bg-slate-900/50 p-2">
              <div className="text-sm font-semibold text-slate-100">
                {profile.totalXp.toLocaleString()}
              </div>
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("hangar.profile.total_xp")}
              </div>
            </div>
            <div className="rounded-lg bg-slate-900/50 p-2">
              <div className="text-sm font-semibold text-slate-100">
                {profile.flightsScored.toLocaleString()}
              </div>
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("hangar.profile.flights")}
              </div>
            </div>
          </div>

          {/* (v7) Acceso a los Logros — el sitio natural: el perfil del piloto. */}
          <button
            onClick={() => {
              setOpen(false);
              setAchOpen(true);
            }}
            className="mt-3 inline-flex w-full items-center justify-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs font-semibold text-amber-200 transition-colors hover:border-amber-500/50 hover:bg-amber-500/15"
          >
            <Trophy className="h-4 w-4" />
            {t("ach.open")}
          </button>
        </div>
      )}

      {achOpen && <AchievementsModal onClose={() => setAchOpen(false)} />}
    </div>
  );
}

export function Avatar({
  avatar,
  identity,
  size,
}: {
  avatar: string | null;
  identity: string;
  size: number;
}) {
  if (avatar) {
    return (
      <img
        src={avatar}
        alt=""
        style={{ width: size, height: size }}
        className="rounded-full object-cover ring-2 ring-brand-500/40"
      />
    );
  }
  const initials = identity === "daniel" ? "D" : "H";
  return (
    <span
      style={{ width: size, height: size, fontSize: size * 0.4 }}
      className="flex items-center justify-center rounded-full bg-gradient-to-br from-brand-500 to-brand-700 font-bold text-white ring-2 ring-brand-500/40"
    >
      {initials}
    </span>
  );
}
