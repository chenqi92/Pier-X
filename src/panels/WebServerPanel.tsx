import type { TabState } from "../lib/types";
import NginxPanel from "./NginxPanel";

// One sidebar entry, one panel — but the underlying product is whichever
// web server the host actually runs. Today only nginx has a real
// implementation, so this is a thin pass-through. When Apache / Caddy
// land, this is where the segmented "nginx · apache · caddy" picker
// will sit (auto-detected, hidden when only one is installed).
type Props = { tab: TabState | null };

export default function WebServerPanel({ tab }: Props) {
  return <NginxPanel tab={tab} />;
}
