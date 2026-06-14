import type { BuilderRule, CompareOp, SellMode } from '../../types/strategy';
import { IndicatorPicker } from '../IndicatorPicker/IndicatorPicker';
import { NumberInput } from '../NumberInput/NumberInput';
import { OptionSlider } from '../OptionSlider/OptionSlider';
import styles from './RuleRow.module.css';

interface RuleRowProps {
  rule: BuilderRule;
  onChange: (patch: Partial<BuilderRule>) => void;
  isExitRule: boolean;
}

const OP_OPTIONS: CompareOp[] = ['<', '=', '>'];
const OP_INDEX: Record<CompareOp, number> = { '<': 0, '=': 1, '>': 2 };

const ENTRY_ACTION_OPTIONS = ['Quantity', 'Money'];
const EXIT_ACTION_OPTIONS = ['Quantity', 'Money', 'All'];

function actionModeToIndex(rule: BuilderRule): number {
  if (rule.actionMode === 'all') return 2;
  if (rule.actionMode === 'money') return 1;
  return 0;
}

function indexToActionMode(rule: BuilderRule, idx: number): BuilderRule['actionMode'] {
  const isExit = rule.actionVerb === 'sell';
  if (isExit) {
    if (idx === 2) return 'all' as SellMode;
    if (idx === 1) return 'money';
    return 'quantity';
  }
  if (idx === 1) return 'money';
  return 'quantity';
}

export function RuleRow({ rule, onChange, isExitRule }: RuleRowProps) {
  const actionOptions = isExitRule ? EXIT_ACTION_OPTIONS : ENTRY_ACTION_OPTIONS;
  const actionIndex = actionModeToIndex(rule);

  return (
    <div className={styles.row}>
      <div className={styles.conditionLine}>
        <span className={styles.label}>If</span>
        <IndicatorPicker
          value={rule.indicator}
          onChange={(indicator) => onChange({ indicator })}
          width={164}
        />
        <NumberInput
          value={rule.period}
          onChange={(period) => onChange({ period: Math.max(1, Math.round(period)) })}
          min={1}
          max={500}
          width={101}
          ariaLabel="indicator period"
        />
        <span className={styles.label}>is</span>
        <OptionSlider
          options={OP_OPTIONS}
          selectedIndex={OP_INDEX[rule.op]}
          onChange={(idx) => onChange({ op: OP_OPTIONS[idx] })}
          width={185}
          ariaLabel="comparison operator"
        />
        <div className={styles.rhsGroup}>
          <OptionSlider
            options={['LTP', 'Value']}
            selectedIndex={rule.rhsMode === 'ltp' ? 0 : 1}
            onChange={(idx) =>
              onChange({ rhsMode: idx === 0 ? 'ltp' : 'value' })
            }
            width={200}
            ariaLabel="right-hand side mode"
          />
          <OptionSlider
            options={['+', '-']}
            selectedIndex={rule.rhsSign === '+' ? 0 : 1}
            onChange={(idx) => onChange({ rhsSign: idx === 0 ? '+' : '-' })}
            width={107}
            ariaLabel="right-hand side sign"
          />
          {rule.rhsMode === 'value' ? (
            <NumberInput
              value={rule.rhsValue}
              onChange={(rhsValue) => onChange({ rhsValue })}
              width={130}
              ariaLabel="threshold value"
            />
          ) : (
            <div className={styles.ltpBadge}>LTP</div>
          )}
        </div>
      </div>

      <div className={styles.actionLine}>
        <span className={styles.actionVerb}>
          {isExitRule ? 'Sell' : 'Buy'}
        </span>
        <NumberInput
          value={rule.actionQuantity}
          onChange={(actionQuantity) =>
            onChange({ actionQuantity: Math.max(1, Math.round(actionQuantity)) })
          }
          min={1}
          width={200}
          ariaLabel="action quantity"
        />
        <OptionSlider
          options={actionOptions}
          selectedIndex={actionIndex}
          onChange={(idx) => onChange({ actionMode: indexToActionMode(rule, idx) })}
          width={240}
          ariaLabel="action mode"
        />
        {isExitRule && (
          <span className={styles.hint}>won&apos;t sell if no holdings</span>
        )}
      </div>
    </div>
  );
}
