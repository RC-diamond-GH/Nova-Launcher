/** Sidebar navigation between the three pages. */

import { el } from "./dom";

export function gotoPage(page: string): void {
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.page === page);
  });
  document.querySelectorAll<HTMLElement>(".page").forEach((p) => {
    p.classList.toggle("active", p.id === `page-${page}`);
  });
}

export function initNav(): void {
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.addEventListener("click", () => gotoPage(b.dataset.page!));
  });
  el<HTMLAnchorElement>("edit-profile").addEventListener("click", (e) => {
    e.preventDefault();
    gotoPage("settings");
  });
}
