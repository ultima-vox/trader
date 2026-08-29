import { mountFoundationPlayground } from "./ui/index";

const root = document.getElementById("app");
if (root instanceof HTMLElement) {
  mountFoundationPlayground(root);
}
