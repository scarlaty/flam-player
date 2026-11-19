import { ToastProvider } from "../../../components";
import { EditorScreen } from "./EditorScreen";

/**
 * Racine de la fenêtre de l'éditeur : aucun des fournisseurs de la fenêtre principale (Flam,
 * bibliothèque, file d'installation) n'y tourne ; tout passe par les commandes de l'éditeur.
 */
export function EditorApp() {
  return (
    <ToastProvider>
      <EditorScreen />
    </ToastProvider>
  );
}
