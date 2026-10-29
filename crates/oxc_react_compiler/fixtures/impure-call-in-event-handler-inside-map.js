// @validateNoImpureFunctionsInRender @outputMode:"lint" @compilationMode:"infer" @panicThreshold:"none"
const ITEMS = [{ id: "a" }, { id: "b" }];

// The handler's impure call only runs after a click, not while rendering the list.
export function HelperCalledFromMapHandler({ onPick }) {
  function handlePick(item) {
    onPick(item.id, Date.now());
  }
  return (
    <div>
      {ITEMS.map((item) => (
        <button key={item.id} onClick={() => handlePick(item)} />
      ))}
    </div>
  );
}

// An inline event handler may itself call an impure function.
export function InlineHandlerInsideMap() {
  return (
    <div>
      {ITEMS.map((item) => (
        <button key={item.id} onClick={() => Date.now()} />
      ))}
    </div>
  );
}

// Passing the handler directly has the same event-time behavior.
export function HandlerPassedDirectlyInsideMap() {
  function handlePick() {
    Date.now();
  }
  return (
    <div>
      {ITEMS.map((item) => (
        <button key={item.id} onClick={handlePick} />
      ))}
    </div>
  );
}

// The same helper is safe when its event wrapper is outside the map callback.
export function HelperCalledOutsideMap({ onPick }) {
  function handlePick(item) {
    onPick(item.id, Date.now());
  }
  return (
    <div>
      <button onClick={() => handlePick(ITEMS[0])} />
    </div>
  );
}

// Initializing a captured local after creating a helper mutates the closures
// that capture it, but does not call those closures during render.
export function HandlerCallsHelperCapturingLaterLocal({ onPick, items }) {
  function readItems() {
    onPick(itemsForHandler);
  }
  function handlePick() {
    readItems();
    onPick(Date.now());
  }
  const itemsForHandler = getItems(items);
  return <Dialog onPick={handlePick} />;
}

// Calls from the map callback itself still execute during render.
export function ImpureCallInMapCallback() {
  return <div>{ITEMS.map((item) => Date.now())}</div>;
}

// A helper called directly from the map callback also runs during render.
export function HelperCalledFromMapCallback() {
  function getTime() {
    return Date.now();
  }
  return <div>{ITEMS.map((item) => getTime())}</div>;
}

// Calling a captured helper through a local alias still runs it during render.
export function AliasedHelperCalledFromMapCallback() {
  function getTime() {
    return Date.now();
  }
  return (
    <div>
      {ITEMS.map(() => {
        const read = getTime;
        return read();
      })}
    </div>
  );
}

// Function.prototype.call and apply invoke their receiver during render too.
export function HelperCalledWithCallFromMapCallback() {
  function getTime() {
    return Date.now();
  }
  return <div>{ITEMS.map(() => getTime.call(null))}</div>;
}

export function HelperCalledWithApplyFromMapCallback() {
  function getTime() {
    return Date.now();
  }
  return <div>{ITEMS.map(() => getTime.apply(null, []))}</div>;
}

// Synchronous callback consumers invoke captured helpers during render too.
export function HelperPassedToNestedMap({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => [1].map(getTime))}</div>;
}

// Synchronously invoked local wrappers preserve their captured call effects.
export function HelperCalledThroughIife({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => (() => getTime())())}</div>;
}

// Synchronously invoked wrappers may forward the called helper as a parameter.
export function HelperForwardedThroughIife({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => ((call) => call())(getTime))}</div>;
}

// Applied and spread argument arrays preserve synchronous callback forwarding.
export function HelperForwardedThroughApply({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => ((call) => call()).apply(null, [getTime]))}</div>;
}

export function HelperForwardedThroughSpread({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => ((call) => call())(...[getTime]))}</div>;
}

// Calling a function-valued object member invokes the captured helper.
export function HelperCalledThroughObjectProperty({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => ({ getTime }).getTime())}</div>;
}

// Calling a bound function invokes the original captured helper.
export function HelperCalledThroughBind({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => getTime.bind(null)())}</div>;
}

// Sparse array-like objects must not allocate through their largest property key.
export function SparseApplyArgumentsDoNotAllocate({ items }) {
  const getTime = () => Date.now();
  return (
    <div>
      {items.map(() =>
        ((call) => 1).apply(null, {
          "18446744073709551615": getTime,
        }),
      )}
    </div>
  );
}

// Forwarding inside an event handler still does not execute during render.
export function HelpersForwardedOnlyInEventHandler({ items }) {
  const getTime = () => Date.now();
  return (
    <div>
      {items.map((item) => (
        <button
          key={item.id}
          onClick={() => {
            ((call) => call())(...[getTime]);
            ({ getTime }).getTime();
            getTime.bind(null)();
          }}
        />
      ))}
    </div>
  );
}

// Bound wrapper arguments execute when the bound wrapper is called during render.
export function HelperForwardedThroughBind({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => ((call) => call()).bind(null, getTime)())}</div>;
}

// A helper returned by a synchronous wrapper is still called during render.
export function HelperReturnedThroughIife({ items }) {
  const getTime = () => Date.now();
  return <div>{items.map(() => (() => getTime)()())}</div>;
}

// Repeated shared spreads must widen instead of expanding exponentially.
export function SharedArraySpreadsConverge({ items }) {
  const getTime = () => Date.now();
  return items.map(() => {
    const a0 = [getTime];
    const a1 = [...a0, ...a0];
    const a2 = [...a1, ...a1];
    const a3 = [...a2, ...a2];
    const a4 = [...a3, ...a3];
    const a5 = [...a4, ...a4];
    const a6 = [...a5, ...a5];
    const a7 = [...a6, ...a6];
    const a8 = [...a7, ...a7];
    const a9 = [...a8, ...a8];
    const a10 = [...a9, ...a9];
    const a11 = [...a10, ...a10];
    const a12 = [...a11, ...a11];
    const a13 = [...a12, ...a12];
    const a14 = [...a13, ...a13];
    const a15 = [...a14, ...a14];
    const a16 = [...a15, ...a15];
    const a17 = [...a16, ...a16];
    const a18 = [...a17, ...a17];
    const a19 = [...a18, ...a18];
    const a20 = [...a19, ...a19];
    const a21 = [...a20, ...a20];
    const a22 = [...a21, ...a21];
    const a23 = [...a22, ...a22];
    const a24 = [...a23, ...a23];
    return ((call) => 1)(...a24);
  });
}
