import { IconHomeBolt, IconServer } from "@tabler/icons-react";
import type { SshHost } from "./picker";

interface Props {
  sshHosts: SshHost[];
  selected: string;
  onSelect: (host: string) => void;
}

export function HostList({ sshHosts, selected, onSelect }: Props) {
  return (
    <aside className="flex w-44 flex-col gap-1 p-2">
      <button
        type="button"
        onClick={() => onSelect("local")}
        className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-sm ${
          selected === "local" ? "bg-accent" : "hover:bg-accent/50"
        }`}
      >
        <IconHomeBolt className="size-4" />
        Local
      </button>
      <div className="my-1 border-t border-border" />
      {sshHosts.length === 0 ? (
        <p className="px-2 py-1 text-xs text-muted-foreground">
          No SSH hosts in ~/.ssh/config
        </p>
      ) : (
        sshHosts.map((h) => (
          <button
            key={h.alias}
            type="button"
            onClick={() => onSelect(h.alias)}
            className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-sm ${
              selected === h.alias ? "bg-accent" : "hover:bg-accent/50"
            }`}
          >
            <IconServer className="size-4" />
            <span className="truncate">{h.alias}</span>
          </button>
        ))
      )}
    </aside>
  );
}
