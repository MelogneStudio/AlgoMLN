import type { BuilderRule } from '../../../../types/strategy';
import { RuleRow } from '../../../../components/RuleRow/RuleRow';
import styles from './RuleSection.module.css';

interface RuleSectionProps {
  type: 'entry' | 'exit';
  rule: BuilderRule;
  onChange: (patch: Partial<BuilderRule>) => void;
}

export function RuleSection({ type, rule, onChange }: RuleSectionProps) {
  return (
    <div className={`${styles.shell} ${styles[type]}`}>
      <span className={styles.header}>{type === 'entry' ? 'Entry' : 'Exit'}</span>
      <div className={styles.inner}>
        <RuleRow
          rule={rule}
          onChange={onChange}
          isExitRule={type === 'exit'}
        />
      </div>
    </div>
  );
}
