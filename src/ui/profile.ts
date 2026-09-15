/** Player card on the home page, driven by the settings username field. */

import { avatar, playerName, usernameInput } from "./dom";

export function syncProfile(): void {
  const name = usernameInput.value.trim() || "Player";
  playerName.textContent = name;
  avatar.textContent = (name[0] ?? "P").toUpperCase();
}
