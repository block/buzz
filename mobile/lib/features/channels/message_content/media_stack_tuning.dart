part of '../message_content.dart';

const _stackTuningEnabled = bool.fromEnvironment('BUZZ_PHOTO_STACK_TUNING');

// Preview-only session settings, shared by message stacks and the tuning sheet.
final _stackTuning = ValueNotifier<List<_StackAdjustment>>(
  List.filled(3, const _StackAdjustment()),
);

Future<void> _openStackTuning(
  BuildContext context,
  List<MessageGalleryItem> items,
  bool mirrored,
) => showModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  useSafeArea: true,
  builder: (_) => FractionallySizedBox(
    heightFactor: .9,
    child: _StackTuningSheet(items: items, mirrored: mirrored),
  ),
);

// Offsets selected in the web tuner, relative to the existing card poses.
const _sentStackAdjustments = [
  _StackAdjustment(),
  // Match the middle card visible in the approved sent-stack reference.
  _StackAdjustment(x: 20, y: -7, degrees: 1),
  _StackAdjustment(x: -18, degrees: -2),
];
const _receivedStackAdjustments = [
  _StackAdjustment(),
  _StackAdjustment(x: 3, y: -7, degrees: 10),
  _StackAdjustment(x: -3, degrees: -15),
];

class _StackAdjustment {
  const _StackAdjustment({this.x = 0, this.y = 0, this.degrees = 0});
  final double x, y, degrees;

  static _StackAdjustment at(List<_StackAdjustment> slots, double depth) {
    final low = depth.floor().clamp(0, 2);
    final high = depth.ceil().clamp(0, 2);
    final t = depth - low;
    final a = slots[low];
    final b = slots[high];
    return _StackAdjustment(
      x: a.x + (b.x - a.x) * t,
      y: a.y + (b.y - a.y) * t,
      degrees: a.degrees + (b.degrees - a.degrees) * t,
    );
  }
}

class _StackTuningSheet extends HookConsumerWidget {
  const _StackTuningSheet({required this.items, required this.mirrored});
  final List<MessageGalleryItem> items;
  final bool mirrored;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final selected = useState(0);
    final settings = useValueListenable(_stackTuning);
    final copied = useState(false);
    final slot = settings[selected.value];
    Widget slider(
      String label,
      double value,
      double max,
      _StackAdjustment Function(double) change,
    ) {
      return Row(
        children: [
          SizedBox(width: 90, child: Text(label)),
          Expanded(
            child: Slider(
              label: value.toStringAsFixed(0),
              value: value,
              min: -max,
              max: max,
              divisions: (max * 2).round(),
              semanticFormatterCallback: (v) => '$label ${v.round()}',
              onChanged: (v) {
                final next = [...settings];
                next[selected.value] = change(v);
                _stackTuning.value = next;
                copied.value = false;
              },
            ),
          ),
          SizedBox(width: 36, child: Text(value.toStringAsFixed(0))),
        ],
      );
    }

    return SafeArea(
      key: const ValueKey('stack-tuning-sheet'),
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(Grid.xs),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('Tune photo stack', style: context.textTheme.titleLarge),
            const Text('Adjust each visible slot. Swipe to try the motion.'),
            Padding(
              padding: const EdgeInsets.symmetric(vertical: Grid.xs),
              child: _MessageImageStack(
                items: items,
                mirrored: mirrored,
                showTuningControl: false,
              ),
            ),
            SegmentedButton<int>(
              segments: [
                for (var i = 0; i < 3; i++)
                  ButtonSegment(value: i, label: Text('Card ${i + 1}')),
              ],
              selected: {selected.value},
              onSelectionChanged: (values) => selected.value = values.single,
            ),
            slider(
              'Horizontal',
              slot.x,
              24,
              (v) => _StackAdjustment(x: v, y: slot.y, degrees: slot.degrees),
            ),
            slider(
              'Vertical',
              slot.y,
              24,
              (v) => _StackAdjustment(x: slot.x, y: v, degrees: slot.degrees),
            ),
            slider(
              'Rotation °',
              slot.degrees,
              15,
              (v) => _StackAdjustment(x: slot.x, y: slot.y, degrees: v),
            ),
            const Text(
              'Offsets from the current design. Kept for this app session.',
            ),
            Wrap(
              alignment: WrapAlignment.center,
              children: [
                TextButton(
                  onPressed: () {
                    _stackTuning.value = List.filled(
                      3,
                      const _StackAdjustment(),
                    );
                    copied.value = false;
                  },
                  child: const Text('Reset all'),
                ),
                TextButton(
                  onPressed: () async {
                    await Clipboard.setData(
                      ClipboardData(
                        text: jsonEncode([
                          for (var i = 0; i < settings.length; i++)
                            {
                              'card': i + 1,
                              'x': settings[i].x,
                              'y': settings[i].y,
                              'rotation': settings[i].degrees,
                            },
                        ]),
                      ),
                    );
                    if (context.mounted) copied.value = true;
                  },
                  child: Text(copied.value ? 'Copied' : 'Copy settings'),
                ),
                TextButton(
                  onPressed: () => Navigator.of(context).pop(),
                  child: const Text('Done'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
