import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createProtocolNamespace, parseProtocolManifest } from "@kybernetria/pi-protocol";

export const manifestPath = fileURLToPath(new URL("../pi.protocol.json", import.meta.url));
export const manifest = parseProtocolManifest(readFileSync(manifestPath, "utf8"));
export const protocolNamespace = createProtocolNamespace(manifest);
