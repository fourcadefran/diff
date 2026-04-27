import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Toaster } from "sonner";
import { HostList } from "./host-list";
import { RecentList } from "./recent-list";

export interface SshHost {
  alias: string;
  hostname?: string;
  user?: string;
}

export function Picker() {
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  const [selectedHost, setSelectedHost] = useState<string>("local");

  useEffect(() => {
    invoke<SshHost[]>("list_ssh_hosts")
      .then(setSshHosts)
      .catch((e) => console.error("[diff-picker] list_ssh_hosts:", e));
  }, []);

  return (
    <main className="flex h-dvh bg-background text-foreground">
      <HostList
        sshHosts={sshHosts}
        selected={selectedHost}
        onSelect={setSelectedHost}
      />
      <div className="flex-1 border-l border-border">
        <RecentList host={selectedHost} />
      </div>
      <Toaster />
    </main>
  );
}
