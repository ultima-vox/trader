import "../../design-system/tokens/tokens.css";
import "../../design-system/primitives/primitives.css";
import "../../design-system/components/components.css";
import "../../design-system/patterns/patterns.css";
import { mountFoundationPlayground } from "./ui/index";

const root = document.getElementById("app");
if (root instanceof HTMLElement) {
  mountFoundationPlayground(root);
}
