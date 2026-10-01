import React, { useState } from "react";
import {
  ZoomIn,
  ZoomOut,
  Maximize2,
  Sliders,
  RotateCcw,
  ChevronDown,
  ChevronUp,
} from "lucide-react";
import type { GraphPhysicsConfig } from "../lib/types";
import { DEFAULT_PHYSICS_CONFIG } from "../lib/physics";

interface BrainPhysicsControlsProps {
  physicsConfig: GraphPhysicsConfig;
  onPhysicsChange: (config: Partial<GraphPhysicsConfig>) => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onRecenter: () => void;
}

export const BrainPhysicsControls: React.FC<BrainPhysicsControlsProps> = ({
  physicsConfig,
  onPhysicsChange,
  onZoomIn,
  onZoomOut,
  onRecenter,
}) => {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <div
      data-testid="brain-physics-controls"
      className="flex flex-col items-end gap-2 text-neutral-200"
    >
      {/* Expanded Sliders Panel */}
      {isOpen && (
        <div className="w-64 p-3 bg-neutral-900/95 backdrop-blur-md border border-neutral-800 rounded-lg shadow-2xl space-y-3 text-xs select-none">
          <div className="flex items-center justify-between pb-1.5 border-b border-neutral-800">
            <span className="font-semibold text-neutral-200">Force Physics</span>
            <button
              onClick={() => onPhysicsChange(DEFAULT_PHYSICS_CONFIG)}
              className="flex items-center gap-1 text-[10px] text-neutral-400 hover:text-neutral-200 transition-colors"
            >
              <RotateCcw className="w-3 h-3" />
              <span>Reset</span>
            </button>
          </div>

          {/* Repulsion Slider */}
          <div className="space-y-1">
            <div className="flex justify-between text-[11px] text-neutral-400">
              <span>Charge Repulsion</span>
              <span>{Math.round(physicsConfig.chargeRepulsion)}</span>
            </div>
            <input
              type="range"
              min="100"
              max="1000"
              value={physicsConfig.chargeRepulsion}
              onChange={(e) =>
                onPhysicsChange({ chargeRepulsion: Number(e.target.value) })
              }
              className="w-full accent-purple-500 h-1 bg-neutral-800 rounded-lg cursor-pointer"
            />
          </div>

          {/* Link Distance Slider */}
          <div className="space-y-1">
            <div className="flex justify-between text-[11px] text-neutral-400">
              <span>Link Distance</span>
              <span>{Math.round(physicsConfig.linkDistance)}px</span>
            </div>
            <input
              type="range"
              min="40"
              max="240"
              value={physicsConfig.linkDistance}
              onChange={(e) =>
                onPhysicsChange({ linkDistance: Number(e.target.value) })
              }
              className="w-full accent-purple-500 h-1 bg-neutral-800 rounded-lg cursor-pointer"
            />
          </div>

          {/* Center Gravity Slider */}
          <div className="space-y-1">
            <div className="flex justify-between text-[11px] text-neutral-400">
              <span>Center Gravity</span>
              <span>{(physicsConfig.centerGravity * 100).toFixed(1)}%</span>
            </div>
            <input
              type="range"
              min="0.005"
              max="0.1"
              step="0.005"
              value={physicsConfig.centerGravity}
              onChange={(e) =>
                onPhysicsChange({ centerGravity: Number(e.target.value) })
              }
              className="w-full accent-purple-500 h-1 bg-neutral-800 rounded-lg cursor-pointer"
            />
          </div>
        </div>
      )}

      {/* Floating Action Buttons */}
      <div className="flex items-center gap-1 p-1 bg-neutral-900/90 backdrop-blur-md border border-neutral-800 rounded-lg shadow-lg">
        <button
          onClick={() => setIsOpen(!isOpen)}
          title="Toggle physics sliders"
          aria-label="Toggle physics sliders"
          className={`flex items-center gap-1 px-2 py-1 text-xs rounded transition-colors ${
            isOpen
              ? "bg-purple-600/25 text-purple-300 border border-purple-500/30"
              : "text-neutral-400 hover:text-neutral-200 hover:bg-neutral-800"
          }`}
        >
          <Sliders className="w-3.5 h-3.5" />
          <span>Physics</span>
          {isOpen ? (
            <ChevronDown className="w-3 h-3 ml-0.5" />
          ) : (
            <ChevronUp className="w-3 h-3 ml-0.5" />
          )}
        </button>

        <div className="w-[1px] h-4 bg-neutral-800 mx-0.5" />

        <button
          onClick={onZoomIn}
          title="Zoom In"
          aria-label="Zoom in"
          className="p-1.5 rounded text-neutral-400 hover:text-neutral-200 hover:bg-neutral-800 transition-colors"
        >
          <ZoomIn className="w-4 h-4" />
        </button>

        <button
          onClick={onZoomOut}
          title="Zoom Out"
          aria-label="Zoom out"
          className="p-1.5 rounded text-neutral-400 hover:text-neutral-200 hover:bg-neutral-800 transition-colors"
        >
          <ZoomOut className="w-4 h-4" />
        </button>

        <button
          onClick={onRecenter}
          title="Recenter Camera"
          aria-label="Recenter camera"
          className="p-1.5 rounded text-neutral-400 hover:text-neutral-200 hover:bg-neutral-800 transition-colors"
        >
          <Maximize2 className="w-4 h-4" />
        </button>
      </div>
    </div>
  );
};
