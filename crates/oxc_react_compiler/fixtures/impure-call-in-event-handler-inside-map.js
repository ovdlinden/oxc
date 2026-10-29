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
          }}
        />
      ))}
    </div>
  );
}
