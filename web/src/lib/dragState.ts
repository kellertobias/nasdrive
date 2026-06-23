import { useEffect } from "react";

export function useGlobalDragCleanup(reset: () => void) {
  useEffect(() => {
    if (typeof window === "undefined") return;

    const handleDragFinished = () => reset();

    window.addEventListener("drop", handleDragFinished, true);
    window.addEventListener("dragend", handleDragFinished, true);
    window.addEventListener("blur", handleDragFinished);

    return () => {
      window.removeEventListener("drop", handleDragFinished, true);
      window.removeEventListener("dragend", handleDragFinished, true);
      window.removeEventListener("blur", handleDragFinished);
    };
  }, [reset]);
}
