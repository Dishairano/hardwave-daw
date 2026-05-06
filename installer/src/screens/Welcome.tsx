import { motion } from "framer-motion";
import { ArrowRight, Music2, Cable, Layers } from "lucide-react";
import { HwLogo } from "../components/HwLogo";
import type { LatestVersion } from "../lib/api";

interface Props {
  version: LatestVersion | null;
  onContinue: () => void;
}

export function Welcome({ version, onContinue }: Props) {
  return (
    <motion.div
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.35 }}
      className="absolute inset-0 flex flex-col px-10 pt-16 pb-8"
    >
      <div className="flex items-start gap-5">
        <HwLogo size={56} glow />
        <div className="flex-1">
          <div className="text-[10px] tracking-[0.28em] text-white/40 uppercase font-medium">
            Welcome to
          </div>
          <h1 className="mt-1 text-[26px] font-bold tracking-tight leading-none">
            Hardwave <span className="hw-accent-text">DAW</span>
          </h1>
          <div className="mt-1 text-xs text-white/50">
            {version ? (
              <>Version {version.version} · The producer's workstation</>
            ) : (
              "The producer's workstation"
            )}
          </div>
        </div>
      </div>

      <div className="mt-7 grid grid-cols-3 gap-3">
        <Feature
          icon={<Music2 size={16} />}
          title="Track everything"
          body="Audio, MIDI, automation, sends — one timeline"
        />
        <Feature
          icon={<Cable size={16} />}
          title="VST3 + CLAP"
          body="Host every plug-in you already own"
        />
        <Feature
          icon={<Layers size={16} />}
          title="Hardwave plug-ins"
          body="Analyser, LoudLab, KickForge ship inside"
        />
      </div>

      <div className="flex-1" />

      <div className="flex items-center justify-between">
        <div className="text-[11px] text-white/40 max-w-[360px] leading-relaxed">
          By continuing you agree to the Hardwave Studios terms of service and
          privacy policy.
        </div>
        <button
          onClick={onContinue}
          className="hw-accent-gradient text-black font-semibold px-6 py-2.5 rounded-lg text-sm flex items-center gap-2 shadow-glow hover:brightness-110 active:scale-[0.98] transition"
        >
          Continue
          <ArrowRight size={16} />
        </button>
      </div>
    </motion.div>
  );
}

function Feature({
  icon,
  title,
  body,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
}) {
  return (
    <div className="bg-white/[0.03] border border-white/[0.06] rounded-lg p-3">
      <div className="flex items-center gap-2 text-hw-accent">
        {icon}
        <div className="text-xs font-semibold text-white">{title}</div>
      </div>
      <div className="mt-1.5 text-[11px] text-white/50 leading-snug">{body}</div>
    </div>
  );
}
