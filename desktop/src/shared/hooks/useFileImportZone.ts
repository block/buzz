import * as React from "react";

type FileImportZoneOptions = {
  /** Called with the raw byte array and original file name. */
  onImportFile: (fileBytes: number[], fileName: string) => void;
};

/**
 * Shared drag-and-drop + file-picker infrastructure for import sections
 * (PersonasSection, TeamsSection). Returns state, handlers, and a ref for
 * the hidden `<input type="file">`.
 */
export function useFileImportZone({ onImportFile }: FileImportZoneOptions) {
  const fileInputRef = React.useRef<HTMLInputElement>(null);
  const [isDragOver, setIsDragOver] = React.useState(false);

  async function importFile(file: File) {
    const buffer = await file.arrayBuffer();
    const bytes = Array.from(new Uint8Array(buffer));
    onImportFile(bytes, file.name);
  }

  const dropHandlers = {
    onDragLeave: () => setIsDragOver(false),
    onDragOver: (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragOver(true);
    },
    onDrop: (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) {
        void importFile(file);
      }
    },
  };

  function handleFileChange(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;

    void importFile(file);

    // Reset so the same file can be re-selected
    e.target.value = "";
  }

  function openFilePicker() {
    fileInputRef.current?.click();
  }

  return {
    fileInputRef,
    isDragOver,
    dropHandlers,
    handleFileChange,
    openFilePicker,
  };
}
