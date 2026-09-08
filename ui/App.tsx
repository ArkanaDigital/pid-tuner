import { useStore } from "./lib/store";
import Home from "./wizard/Home";
import Wizard from "./wizard/Wizard";
import QuickLook from "./QuickLook";
import Settings from "./settings/Settings";

export default function App() {
  const view = useStore((s) => s.view);
  if (view === "wizard") return <Wizard />;
  if (view === "quick") return <QuickLook />;
  if (view === "settings") return <Settings />;
  return <Home />;
}
