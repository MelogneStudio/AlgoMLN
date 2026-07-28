import { useCallback, useEffect, useMemo, useState } from 'react';
import { AppWindow } from './components/AppWindow/AppWindow';
import { TitleBar } from './components/TitleBar/TitleBar';
import { Sidebar } from './components/Sidebar/Sidebar';
import { BuilderScreen } from './screens/Builder/BuilderScreen';
import { StrategyCoderScreen } from './screens/StrategyCoder/StrategyCoderScreen';
import { StrategyUploaderScreen } from './screens/StrategyUploader/StrategyUploaderScreen';
import { StrategiesScreen } from './screens/Strategies/StrategiesScreen';
import { SettingsScreen } from './screens/Settings/SettingsScreen';
import { PluginsScreen } from './screens/Plugins/PluginsScreen';
import { LiveScreen } from './screens/Live/LiveScreen';
import { LiveConfirmModal } from './components/LiveConfirmModal/LiveConfirmModal';
import { useStrategyBuilder } from './hooks/useStrategyBuilder';
import { useBacktest } from './hooks/useBacktest';
import { strategyToDsl, useDslSync } from './hooks/useDslSync';
import {
  applyScale,
  computeFitScale,
  getScreenSize,
  loadSavedCapital,
  SIDEBAR_FORCE_COLLAPSE_THRESHOLD,
} from './lib/scaling';
import { isTauri, validateDsl, listen } from './types/tauri';
import type { BuilderRule } from './types/strategy';
import type { LiveSessionFailedPayload, LiveSessionStoppedPayload } from './types/live';
import { useToast } from './components/Toast/ToastContext';
import styles from './App.module.css';

export type Screen = 'builder' | 'strategies' | 'plugins' | 'settings' | 'live';
export type Modal = 'none' | 'uploader' | 'coder';

export function App() {
  // ----- Scale (lives in App because multiple components read it) -----
  // Computed once from the screen on launch and never changed. There is no
  // user-facing scale control: the app fits itself to the screen and stays.
  const scale = useMemo(() => {
    const { w, h } = getScreenSize();
    return computeFitScale(w, h);
  }, []);

  // Size + center the OS window to the scaled canvas, once on mount.
  useEffect(() => {
    void applyScale(scale);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ----- Sidebar collapse lock from scale -----
  const [sidebarUserCollapsed, setSidebarUserCollapsed] = useState(false);
  const scaleForcesCollapse = scale < SIDEBAR_FORCE_COLLAPSE_THRESHOLD;
  const isSidebarCollapsed = scaleForcesCollapse || sidebarUserCollapsed;
  const canToggleSidebar = !scaleForcesCollapse;

  // ----- Screen + modal state -----
  const [screen, setScreen] = useState<Screen>('builder');
  const [modal, setModal] = useState<Modal>('none');

  // ----- Builder state -----
  const {
    strategy,
    isAdvancedMode,
    setEntryRule,
    setExitRule,
    resetStrategy,
    loadFromDsl,
  } = useStrategyBuilder();

  const { dsl, isValid: dslIsValid, validationErrors } = useDslSync(strategy);
  const backtest = useBacktest();

  // ----- Backtest config -----
  const [backtestSymbol, setBacktestSymbol] = useState('RELIANCE');
  const [backtestCapital, setBacktestCapital] = useState<number>(() =>
    loadSavedCapital()
  );

  // ----- Coder state (the editor's current source text) -----
  const [coderSource, setCoderSource] = useState<string>('');
  const [coderReadOnly, setCoderReadOnly] = useState(false);
  const [coderError, setCoderError] = useState<string | null>(null);

  // ----- Strategies refresh tick -----
  const [strategiesRefreshKey, setStrategiesRefreshKey] = useState(0);
  const bumpStrategies = useCallback(() => {
    setStrategiesRefreshKey((k) => k + 1);
  }, []);

  // ----- Live confirm modal state -----
  const [liveConfirmStrategyId, setLiveConfirmStrategyId] = useState<string | null>(null);
  const [liveConfirmStrategyName, setLiveConfirmStrategyName] = useState<string>('');

  // ----- Toast + Live event listeners -----
  const { showToast } = useToast();

  useEffect(() => {
    if (!isTauri()) return;
    const unlistenFail = listen<LiveSessionFailedPayload>(
      'live_session_failed',
      (event) => {
        showToast({
          kind: 'error',
          title: 'Live session failed',
          body: `${event.payload.reason}. Open positions may need manual attention.`,
          sticky: true,
        });
      }
    );
    const unlistenStop = listen<LiveSessionStoppedPayload>(
      'live_session_stopped_with_positions',
      (event) => {
        showToast({
          kind: 'warning',
          title: 'Session stopped',
          body: event.payload.warning,
          sticky: true,
        });
      }
    );
    return () => {
      unlistenFail.then((f) => f());
      unlistenStop.then((f) => f());
    };
  }, [showToast]);

  // ----- Coder open behaviour -----
  const openCoderFromBuilder = useCallback(() => {
    setCoderSource(strategyToDsl(strategy));
    setCoderReadOnly(false);
    setCoderError(null);
    setModal('coder');
  }, [strategy]);

  const openCoderReadOnly = useCallback((source: string) => {
    setCoderSource(source);
    setCoderReadOnly(true);
    setCoderError(null);
    setModal('coder');
  }, []);

  // ----- Done handler for coder -----
  const handleCoderDone = useCallback(
    async (source: string) => {
      if (isTauri()) {
        try {
          const errs = await validateDsl(source);
          if (errs.length > 0) {
            setCoderError(errs.join('; '));
            return;
          }
        } catch (err) {
          setCoderError(err instanceof Error ? err.message : String(err));
          return;
        }
      }
      const ok = loadFromDsl(source);
      if (ok) {
        setCoderError(null);
        setModal('none');
      } else {
        setCoderError(
          'Strategy uses features the visual builder cannot represent. Edit in the coder.'
        );
      }
    },
    [loadFromDsl]
  );

  // ----- Run backtest -----
  const runBacktest = useCallback(() => {
    void backtest.run(dsl, backtestSymbol, backtestCapital);
  }, [backtest, dsl, backtestSymbol, backtestCapital]);

  const onOpenUploader = useCallback(() => {
    setModal('uploader');
  }, []);

  const onCloseModal = useCallback(() => {
    setModal('none');
    setCoderError(null);
  }, []);

  const onLoadFromUploader = useCallback((source: string) => {
    setCoderSource(source);
    setCoderReadOnly(false);
    setCoderError(null);
    setModal('coder');
  }, []);

  const onViewCodeFromStrategyCard = useCallback(
    (source: string, _name: string) => {
      openCoderReadOnly(source);
    },
    [openCoderReadOnly]
  );

  const onGoLive = useCallback((id: string, name: string) => {
    setLiveConfirmStrategyId(id);
    setLiveConfirmStrategyName(name);
  }, []);

  const onNavigateToLive = useCallback(() => {
    setScreen('live');
    setLiveConfirmStrategyId(null);
  }, []);

  const onRuleChange = useCallback(
    (side: 'entry' | 'exit', patch: Partial<BuilderRule>) => {
      if (side === 'entry') setEntryRule(patch);
      else setExitRule(patch);
    },
    [setEntryRule, setExitRule]
  );

  return (
    <AppWindow scale={scale}>
      <TitleBar
        sidebarCollapsed={isSidebarCollapsed}
        onToggleSidebar={() => setSidebarUserCollapsed((v) => !v)}
        canToggle={canToggleSidebar}
      />
      <div className={styles.content}>
        <Sidebar
          collapsed={sidebarUserCollapsed}
          forcedCollapsed={scaleForcesCollapse}
          scale={scale}
          active={screen}
          onNavigate={setScreen}
        />
        <div className={styles.screenArea}>
          {screen === 'builder' && (
            <BuilderScreen
              strategy={strategy}
              isAdvancedMode={isAdvancedMode || !dslIsValid}
              onEntryChange={(patch) => onRuleChange('entry', patch)}
              onExitChange={(patch) => onRuleChange('exit', patch)}
              onOpenCoder={openCoderFromBuilder}
              onOpenUploader={onOpenUploader}
              onRunBacktest={runBacktest}
              onReset={resetStrategy}
              backtest={backtest}
              backtestSymbol={backtestSymbol}
              backtestCapital={backtestCapital}
              onSymbolChange={setBacktestSymbol}
              onCapitalChange={setBacktestCapital}
            />
          )}
          {screen === 'strategies' && (
            <StrategiesScreen
              refreshKey={strategiesRefreshKey}
              onViewCode={onViewCodeFromStrategyCard}
              onChanged={bumpStrategies}
              onGoLive={onGoLive}
            />
          )}
          {screen === 'plugins' && <PluginsScreen />}
          {screen === 'settings' && <SettingsScreen />}
          {screen === 'live' && <LiveScreen />}
        </div>
      </div>

      <StrategyCoderScreen
        open={modal === 'coder'}
        initialSource={coderSource}
        onClose={onCloseModal}
        onSave={handleCoderDone}
        readOnly={coderReadOnly}
        error={coderError}
      />

      <StrategyUploaderScreen
        open={modal === 'uploader'}
        onClose={onCloseModal}
        onOpenEditor={() => {
          setCoderSource(strategyToDsl(strategy));
          setCoderReadOnly(false);
          setCoderError(null);
        }}
        onLoadSource={onLoadFromUploader}
      />

      {liveConfirmStrategyId && (
        <LiveConfirmModal
          strategyId={liveConfirmStrategyId}
          strategyName={liveConfirmStrategyName}
          onSuccess={onNavigateToLive}
          onCancel={() => {
            setLiveConfirmStrategyId(null);
            setLiveConfirmStrategyName('');
          }}
        />
      )}

      {/* Visible when there are validation errors from the live DSL */}
      {validationErrors.length > 0 && screen === 'builder' && (
        <div className={styles.validationToast} role="status">
          {validationErrors[0]}
        </div>
      )}
    </AppWindow>
  );
}

