import { useMemo } from "react";
import { useSimBriefStore } from "../stores/useSimBriefStore";

/** Un OFP más viejo que esto ya no es el "próximo vuelo". */
export const NEXT_OFP_MAX_AGE_MS = 36 * 60 * 60 * 1000;

/** OFP de SimBrief más reciente si tiene menos de 36 h ("próximo vuelo").
 *  (8.4.x) Compartido por Inicio y la pantalla «En vuelo». */
export function useLatestOfp() {
  const ofps = useSimBriefStore((s) => s.flights);
  return useMemo(() => {
    let best: (typeof ofps)[number] | null = null;
    let bestTs = 0;
    for (const f of ofps) {
      const ts = f.generatedAt ? parseInt(f.generatedAt, 10) * 1000 : 0;
      if (ts > bestTs) {
        best = f;
        bestTs = ts;
      }
    }
    return best && Date.now() - bestTs < NEXT_OFP_MAX_AGE_MS ? best : null;
  }, [ofps]);
}
