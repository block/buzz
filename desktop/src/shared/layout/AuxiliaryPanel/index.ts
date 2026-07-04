export { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanelShell";
export { AuxiliaryPanelBody } from "@/shared/layout/AuxiliaryPanelBody";
export {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
  AuxiliaryPanelTitle,
  type AuxiliaryPanelMode,
  getAuxiliaryPanelBodyClass,
  getAuxiliaryPanelMode,
} from "@/shared/layout/AuxiliaryPanelHeader";
export {
  AuxiliaryPanelContext,
  requireAuxiliaryPanelContext,
  resolveAuxiliaryPanelBodyMode,
  useAuxiliaryPanel,
} from "@/shared/layout/auxiliaryPanelContext";
export type {
  AuxiliaryPanelContextValue,
  AuxiliaryPanelLayout,
} from "@/shared/layout/auxiliaryPanelContext";
export {
  AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
  AUXILIARY_PANEL_MAX_WIDTH_PX,
  AUXILIARY_PANEL_MIN_WIDTH_PX,
  AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX,
  clampAuxiliaryPanelWidth,
  getAuxiliaryPanelMaxWidth,
} from "@/shared/layout/auxiliaryPanelLayout";
