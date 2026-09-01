import "../../design-system/tokens/tokens.css";
import "../../design-system/primitives/primitives.css";
import "../../design-system/components/components.css";
import "../../design-system/patterns/patterns.css";
import { mountApplication } from "./ui/application";

const root = document.getElementById("app");
if (root instanceof HTMLElement) {
  mountApplication(root);
}
