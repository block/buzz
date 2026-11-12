part of '../pairing_page.dart';

// Port of buzz.xyz's BeeSwarm flight model (2026-10-31): wandering heading,
// periodic darting, separation, home attraction, and soft edge turns.
class _WebsiteBee {
  Offset position;
  Offset velocity = Offset.zero;
  Offset home;
  Offset entrance;
  final double seed;
  double heading;
  double rotation;

  _WebsiteBee(Size size, int index)
    : seed = ((index + 1) * 0.7548776662) % 1,
      position = Offset(
        (0.5 + (index + 1) / 1.324717957244746) % 1 * size.width,
        (0.5 + (index + 1) / math.pow(1.324717957244746, 2)) % 1 * size.height,
      ),
      home = Offset(
        (0.12 + ((0.5 + (index + 1) / 1.324717957244746) % 1) * 0.76) *
            size.width,
        (0.14 +
                ((0.5 + (index + 1) / math.pow(1.324717957244746, 2)) % 1) *
                    0.72) *
            size.height,
      ),
      entrance = Offset(
        size.width / 2 +
            math.cos(
                  ((index + 1) * 0.7548776662 % 1) * math.pi * 2 + index * 0.37,
                ) *
                (size.width * 0.7 + 46),
        size.height / 2 +
            math.sin(
                  ((index + 1) * 0.7548776662 % 1) * math.pi * 2 + index * 0.37,
                ) *
                (size.height * 0.7 + 31),
      ),
      heading = ((index + 1) * 0.7548776662 % 1) * math.pi * 2,
      rotation = ((index + 1) * 0.7548776662 % 1) * math.pi * 2 + math.pi / 2;
}

class _WebsiteBeeMotion {
  Size size;
  late final bees = List.generate(12, (index) => _WebsiteBee(size, index));
  double elapsed = 0;

  _WebsiteBeeMotion(this.size);

  // Preserve each bee and the flight clock across rotation/window resizing.
  // Mapping positions to the new bounds avoids replaying the entrance or
  // leaving a portrait swarm outside a shorter landscape viewport.
  void resize(Size nextSize) {
    if (nextSize == size || nextSize.isEmpty || size.isEmpty) return;
    Offset remap(Offset point) => Offset(
      point.dx / size.width * nextSize.width,
      point.dy / size.height * nextSize.height,
    );
    for (final bee in bees) {
      bee.position = remap(bee.position);
      bee.home = remap(bee.home);
      bee.entrance = remap(bee.entrance);
    }
    size = nextSize;
  }

  void advance(double dt, Offset? pointer) {
    elapsed += dt;
    final positions = bees.map((bee) => bee.position).toList();
    for (var index = 0; index < bees.length; index++) {
      final bee = bees[index];
      final p = bee.position;
      final seed = bee.seed;
      const tau = math.pi * 2;
      var heading =
          bee.heading +
          (math.sin(elapsed * (0.27 + seed * 0.4) + seed * tau) +
                  0.6 * math.sin(elapsed * (0.13 + seed * 0.11) + seed * 7)) *
              1.15 *
              dt;
      double edge(double value, double extent) => value < 110
          ? 1 - value / 110
          : value > extent - 110
          ? -(1 - (extent - value) / 110)
          : 0;
      final turn = Offset(edge(p.dx, size.width), edge(p.dy, size.height));
      if (turn != Offset.zero) {
        final delta = (turn.direction - heading + math.pi * 3) % tau - math.pi;
        heading += delta * 3.2 * math.min(1, turn.distance) * dt;
      }
      final phase = (elapsed * (0.24 + seed * 0.2) + seed * 5.37) % 1;
      final burst = phase < 0.18 ? math.sin(phase / 0.18 * math.pi) : 0.0;
      final speed = math.max(
        58 * 0.46,
        58 *
                (1 +
                    math.sin(elapsed * (0.42 + seed * 0.28) + seed * tau) *
                        0.32) +
            math.max(1, size.width) * 1.1 * burst,
      );
      final target = Offset(math.cos(heading), math.sin(heading)) * speed;
      var velocity =
          bee.velocity + (target - bee.velocity) * (2.4 + 11 * burst) * dt;
      for (var other = 0; other < positions.length; other++) {
        if (other == index) continue;
        final delta = p - positions[other];
        final distance = delta.distance;
        if (distance > 0 && distance < 66) {
          velocity += delta * (1300 * (1 - distance / 66) / distance) * dt;
        }
      }
      if (pointer != null) {
        final delta = p - pointer;
        final distance = delta.distance;
        if (distance > 0 && distance < 380) {
          velocity += delta * (1600 * (1 - distance / 380) / distance) * dt;
        }
      }
      velocity += (bee.home - p) * 0.18 * dt;
      velocity += turn * 900 * dt;
      velocity *= 0.97;
      var next = p + velocity * dt;
      if (next.dx < 0 || next.dx > size.width) {
        velocity = Offset(-velocity.dx * 0.5, velocity.dy);
      }
      if (next.dy < 0 || next.dy > size.height) {
        velocity = Offset(velocity.dx, -velocity.dy * 0.5);
      }
      next = Offset(
        next.dx.clamp(0, size.width),
        next.dy.clamp(0, size.height),
      );
      bee.position = next;
      bee.velocity = velocity;
      bee.heading = heading;
      if (velocity.distance > 6) {
        final delta =
            (velocity.direction + math.pi / 2 - bee.rotation + math.pi * 3) %
                tau -
            math.pi;
        bee.rotation += delta * 0.18;
      }
    }
  }
}
