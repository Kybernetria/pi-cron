import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { parseProtocolManifest } from "@kybernetria/pi-protocol/contract";

export const manifestPath = fileURLToPath(new URL("../pi.protocol.json", import.meta.url));
export const definition = parseProtocolManifest(readFileSync(manifestPath, "utf8"), { allowLegacyV02: false });
export const protocolNodeId = definition.manifest.node.id;
