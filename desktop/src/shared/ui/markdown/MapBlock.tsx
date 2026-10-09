import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import * as React from "react";

import { useTheme } from "@/shared/theme/ThemeProvider";

import { type MapSpec, mapSpecPoints, parseMapSpec } from "./mapSpec";

// OpenFreeMap: free vector tiles, no API key.
const STYLE_URL = {
  light: "https://tiles.openfreemap.org/styles/liberty",
  dark: "https://tiles.openfreemap.org/styles/dark",
};
const DEFAULT_COLOR = "#3b82f6";
const DEFAULT_HEIGHT = 360;

function safeColor(color: string | undefined) {
  return color && CSS.supports("color", color) ? color : DEFAULT_COLOR;
}

function addOverlays(map: maplibregl.Map, spec: MapSpec) {
  for (const marker of spec.markers) {
    const pin = new maplibregl.Marker({ color: safeColor(marker.color) })
      .setLngLat([marker.lng, marker.lat])
      .addTo(map);
    if (marker.label) {
      pin.getElement().title = marker.label;
      pin.setPopup(new maplibregl.Popup({ offset: 24 }).setText(marker.label));
    }
  }
  if (spec.lines.length === 0) return;
  map.addSource("buzz-lines", {
    type: "geojson",
    data: {
      type: "FeatureCollection",
      features: spec.lines.map((line) => ({
        type: "Feature",
        properties: { color: safeColor(line.color) },
        geometry: { type: "LineString", coordinates: line.coordinates },
      })),
    },
  });
  map.addLayer({
    id: "buzz-lines",
    type: "line",
    source: "buzz-lines",
    layout: { "line-cap": "round", "line-join": "round" },
    paint: { "line-color": ["get", "color"], "line-width": 4 },
  });
}

function MapView({ spec }: { spec: MapSpec }) {
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const { isDark } = useTheme();

  React.useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const points = mapSpecPoints(spec);
    const map = new maplibregl.Map({
      container,
      style: STYLE_URL[isDark ? "dark" : "light"],
      center: spec.center ?? points[0],
      zoom: spec.zoom ?? 12,
      attributionControl: { compact: true },
      cooperativeGestures: true,
    });
    map.addControl(new maplibregl.NavigationControl(), "top-right");
    if (!spec.center && points.length > 1) {
      const bounds = new maplibregl.LngLatBounds(points[0], points[0]);
      for (const point of points) bounds.extend(point);
      map.fitBounds(bounds, {
        padding: 48,
        maxZoom: spec.zoom ?? 15,
        duration: 0,
      });
    }
    map.on("load", () => addOverlays(map, spec));
    return () => map.remove();
  }, [spec, isDark]);

  return (
    <div
      className="overflow-hidden rounded-2xl border border-border/70"
      data-testid="markdown-map-block"
      ref={containerRef}
      style={{ height: spec.height ?? DEFAULT_HEIGHT }}
    />
  );
}

export default function MapBlock({ code }: { code: string }) {
  const result = React.useMemo(() => parseMapSpec(code), [code]);
  if (!result.ok) {
    return (
      <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {result.error}
      </p>
    );
  }
  return <MapView spec={result.spec} />;
}
