import mapboxgl from "mapbox-gl";
import "mapbox-gl/dist/mapbox-gl.css";
import * as React from "react";

import { useTheme } from "@/shared/theme/ThemeProvider";

import { type MapSpec, mapSpecPoints, parseMapSpec } from "./mapSpec";

// Same Mapbox styles as wondershop. Public (pk.) token from VITE_MAPBOX_TOKEN.
const MAPBOX_TOKEN: string | undefined = import.meta.env.VITE_MAPBOX_TOKEN;
const STYLE_URL = {
  light: "mapbox://styles/mapbox/light-v11",
  dark: "mapbox://styles/mapbox/dark-v11",
};
const DEFAULT_COLOR = "#3b82f6";
const DEFAULT_HEIGHT = 360;

function safeColor(color: string | undefined) {
  return color && CSS.supports("color", color) ? color : DEFAULT_COLOR;
}

function addOverlays(map: mapboxgl.Map, spec: MapSpec) {
  for (const marker of spec.markers) {
    const pin = new mapboxgl.Marker({ color: safeColor(marker.color) })
      .setLngLat([marker.lng, marker.lat])
      .addTo(map);
    if (marker.label) {
      pin.getElement().title = marker.label;
      pin.setPopup(new mapboxgl.Popup({ offset: 24 }).setText(marker.label));
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
    const map = new mapboxgl.Map({
      accessToken: MAPBOX_TOKEN,
      container,
      style: STYLE_URL[isDark ? "dark" : "light"],
      center: spec.center ?? points[0],
      zoom: spec.zoom ?? 12,
      attributionControl: false,
      cooperativeGestures: true,
    });
    map.addControl(new mapboxgl.AttributionControl({ compact: true }));
    map.addControl(new mapboxgl.NavigationControl(), "top-right");
    if (!spec.center && points.length > 1) {
      const bounds = new mapboxgl.LngLatBounds(points[0], points[0]);
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

function MapError({ children }: { children: React.ReactNode }) {
  return (
    <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      {children}
    </p>
  );
}

export default function MapBlock({ code }: { code: string }) {
  const result = React.useMemo(() => parseMapSpec(code), [code]);
  if (!result.ok) return <MapError>{result.error}</MapError>;
  if (!MAPBOX_TOKEN) return <MapError>Map needs VITE_MAPBOX_TOKEN.</MapError>;
  return <MapView spec={result.spec} />;
}
