/** Memory slider / number field, kept in sync with each other. */

import { memLabel, memNum, memRange } from "./dom";

const MIN_MEMORY_MB = 512;
const MAX_MEMORY_MB = 65536;
const DEFAULT_MEMORY_MB = 2048;

/** The range input stops at 16384; the number field is the escape hatch. */
export function clampMemory(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return DEFAULT_MEMORY_MB;
  return Math.min(MAX_MEMORY_MB, Math.max(MIN_MEMORY_MB, Math.floor(value)));
}

export function setMemory(mb: number): void {
  memRange.value = String(clampMemory(mb));
  memNum.value = memRange.value;
  memLabel.textContent = `${memRange.value} MB`;
}
