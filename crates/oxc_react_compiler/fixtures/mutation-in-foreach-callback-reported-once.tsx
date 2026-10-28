function useMetrics(requestConfig) {
  const {customFilters} = requestConfig;

  if (customFilters) {
    Object.keys(customFilters).forEach(key => {
      customFilters[key] = {...customFilters[key]};
    });
  }

  return customFilters;
}
