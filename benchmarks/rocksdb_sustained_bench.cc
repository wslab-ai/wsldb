#include <algorithm>
#include <array>
#include <bit>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <exception>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

#include "rocksdb/cache.h"
#include "rocksdb/db.h"
#include "rocksdb/options.h"
#include "rocksdb/statistics.h"
#include "rocksdb/table.h"
#include "rocksdb/version.h"
#include "rocksdb/write_batch.h"

namespace {

using Clock = std::chrono::steady_clock;
using Nanoseconds = std::chrono::nanoseconds;

constexpr std::uint64_t kSeed = 0x4D595DF4D0F33173ULL;
constexpr std::uint64_t kLoadBatch = 10000;
constexpr std::size_t kDefaultWriteBufferBytes = 64 * 1024 * 1024;

constexpr std::array<std::uint32_t, 8> kSha256Initial = {
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
};

constexpr std::array<std::uint32_t, 64> kSha256Rounds = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b,
    0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
    0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7,
    0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152,
    0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
    0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
    0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
    0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
};

class Sha256 {
 public:
  void Update(const char* data, std::size_t size) {
    bytes_ += size;
    if (buffered_ != 0) {
      const auto copied = std::min(size, buffer_.size() - buffered_);
      std::copy_n(data, copied, buffer_.begin() + buffered_);
      buffered_ += copied;
      data += copied;
      size -= copied;
      if (buffered_ == buffer_.size()) {
        Compress(buffer_);
        buffered_ = 0;
      } else {
        return;
      }
    }
    while (size >= buffer_.size()) {
      std::array<std::uint8_t, 64> block{};
      std::copy_n(data, block.size(), block.begin());
      Compress(block);
      data += block.size();
      size -= block.size();
    }
    std::copy_n(data, size, buffer_.begin());
    buffered_ = size;
  }

  std::array<std::uint8_t, 32> Finalize() {
    const auto bit_length = bytes_ * 8;
    buffer_[buffered_++] = 0x80;
    if (buffered_ > 56) {
      std::fill(buffer_.begin() + buffered_, buffer_.end(), 0);
      Compress(buffer_);
      buffer_.fill(0);
    } else {
      std::fill(buffer_.begin() + buffered_, buffer_.begin() + 56, 0);
    }
    for (std::size_t offset = 0; offset < 8; ++offset) {
      buffer_[56 + offset] = static_cast<std::uint8_t>(
          bit_length >> static_cast<unsigned>((7 - offset) * 8));
    }
    Compress(buffer_);
    std::array<std::uint8_t, 32> digest{};
    for (std::size_t word = 0; word < state_.size(); ++word) {
      for (std::size_t byte = 0; byte < 4; ++byte) {
        digest[word * 4 + byte] = static_cast<std::uint8_t>(
            state_[word] >> static_cast<unsigned>((3 - byte) * 8));
      }
    }
    return digest;
  }

 private:
  void Compress(const std::array<std::uint8_t, 64>& block) {
    std::array<std::uint32_t, 64> schedule{};
    for (std::size_t word = 0; word < 16; ++word) {
      for (std::size_t byte = 0; byte < 4; ++byte) {
        schedule[word] |= static_cast<std::uint32_t>(block[word * 4 + byte])
                          << static_cast<unsigned>((3 - byte) * 8);
      }
    }
    for (std::size_t index = 16; index < schedule.size(); ++index) {
      const auto first = std::rotr(schedule[index - 15], 7) ^
                         std::rotr(schedule[index - 15], 18) ^
                         (schedule[index - 15] >> 3);
      const auto second = std::rotr(schedule[index - 2], 17) ^
                          std::rotr(schedule[index - 2], 19) ^
                          (schedule[index - 2] >> 10);
      schedule[index] = schedule[index - 16] + first + schedule[index - 7] +
                        second;
    }
    auto [a, b, c, d, e, f, g, h] = state_;
    for (std::size_t index = 0; index < schedule.size(); ++index) {
      const auto choice = (e & f) ^ (~e & g);
      const auto majority = (a & b) ^ (a & c) ^ (b & c);
      const auto first = h +
                         (std::rotr(e, 6) ^ std::rotr(e, 11) ^
                          std::rotr(e, 25)) +
                         choice + kSha256Rounds[index] + schedule[index];
      const auto second =
          (std::rotr(a, 2) ^ std::rotr(a, 13) ^ std::rotr(a, 22)) + majority;
      h = g;
      g = f;
      f = e;
      e = d + first;
      d = c;
      c = b;
      b = a;
      a = first + second;
    }
    const std::array<std::uint32_t, 8> compressed = {a, b, c, d, e, f, g, h};
    for (std::size_t index = 0; index < state_.size(); ++index) {
      state_[index] += compressed[index];
    }
  }

  std::array<std::uint32_t, 8> state_ = kSha256Initial;
  std::array<std::uint8_t, 64> buffer_{};
  std::size_t buffered_ = 0;
  std::uint64_t bytes_ = 0;
};

struct Config {
  std::string mode;
  std::filesystem::path database;
  std::uint64_t entries;
  std::size_t value_bytes;
  std::uint64_t duration_seconds;
  std::size_t cache_bytes;
  std::size_t write_buffer_bytes;
  std::uint64_t maintenance_seconds;
  std::string distribution;
  double zipf_exponent;
  std::uint64_t operation_limit;
  bool report_wal_statistics;
};

[[noreturn]] void Fail(const std::string& message) {
  throw std::runtime_error(message);
}

std::uint64_t ParseU64(const char* text, const char* name) {
  if (text[0] == '\0' || text[0] == '-') {
    Fail(std::string(name) + " must be a positive integer");
  }
  std::size_t parsed = 0;
  const auto value = std::stoull(text, &parsed);
  if (text[parsed] != '\0' || value == 0) {
    Fail(std::string(name) + " must be a positive integer");
  }
  return value;
}

double ParseDouble(const char* text, const char* name) {
  std::size_t parsed = 0;
  const auto value = std::stod(text, &parsed);
  if (text[parsed] != '\0' || !std::isfinite(value) || value <= 0.0) {
    Fail(std::string(name) + " must be a positive finite number");
  }
  return value;
}

Config ParseConfig(int argc, char** argv) {
  if (argc != 10) {
    Fail("usage: rocksdb_sustained_bench MODE DB ENTRIES VALUE_BYTES "
         "DURATION_SECONDS CACHE_BYTES MAINTENANCE_SECONDS DISTRIBUTION "
         "ZIPF_EXPONENT");
  }
  const auto* operation_limit = std::getenv("WSLDB_BENCH_OPERATION_LIMIT");
  const auto* write_buffer = std::getenv("WSLDB_BENCH_WRITE_BUFFER_BYTES");
  Config config{
      argv[1],
      argv[2],
      ParseU64(argv[3], "ENTRIES"),
      static_cast<std::size_t>(ParseU64(argv[4], "VALUE_BYTES")),
      ParseU64(argv[5], "DURATION_SECONDS"),
      static_cast<std::size_t>(ParseU64(argv[6], "CACHE_BYTES")),
      write_buffer == nullptr
          ? kDefaultWriteBufferBytes
          : static_cast<std::size_t>(
                ParseU64(write_buffer, "WSLDB_BENCH_WRITE_BUFFER_BYTES")),
      ParseU64(argv[7], "MAINTENANCE_SECONDS"),
      argv[8],
      ParseDouble(argv[9], "ZIPF_EXPONENT"),
      operation_limit == nullptr
          ? 0
          : ParseU64(operation_limit, "WSLDB_BENCH_OPERATION_LIMIT"),
      std::getenv("WSLDB_ROCKSDB_WAL_STATS") != nullptr,
  };
  if (config.mode != "load" && config.mode != "run") {
    Fail("MODE must be load or run");
  }
  if (config.distribution != "uniform" && config.distribution != "zipf") {
    Fail("DISTRIBUTION must be uniform or zipf");
  }
  return config;
}

void Require(const rocksdb::Status& status, std::string_view operation) {
  if (!status.ok()) {
    Fail(std::string(operation) + ": " + status.ToString());
  }
}

std::uint64_t XorShift(std::uint64_t value) {
  value ^= value << 13;
  value ^= value >> 7;
  return value ^ (value << 17);
}

std::uint64_t RotateLeft(std::uint64_t value, unsigned amount) {
  return (value << amount) | (value >> (64 - amount));
}

std::array<char, 16> Key(std::uint64_t index) {
  std::array<char, 16> bytes{};
  for (std::size_t offset = 0; offset < 8; ++offset) {
    const auto shift = static_cast<unsigned>((7 - offset) * 8);
    bytes[offset] = static_cast<char>((index >> shift) & 0xff);
  }
  std::fill(bytes.begin() + 8, bytes.end(), '0');
  return bytes;
}

void FillValue(std::uint64_t index, std::string* value) {
  auto random = index ^ kSeed;
  for (char& byte : *value) {
    random = XorShift(random);
    byte = static_cast<char>(random & 0xff);
  }
}

double H(double value, double exponent) {
  return std::exp(-exponent * std::log(value));
}

double Expm1OverX(double value) {
  if (std::abs(value) > 1.0e-8) {
    return std::expm1(value) / value;
  }
  return 1.0 + value * (0.5 + value * (1.0 / 6.0 + value / 24.0));
}

double Log1pOverX(double value) {
  if (std::abs(value) > 1.0e-8) {
    return std::log1p(value) / value;
  }
  return 1.0 - value * (0.5 - value * (1.0 / 3.0 - value / 4.0));
}

double HIntegral(double value, double exponent) {
  const auto log = std::log(value);
  return log * Expm1OverX((1.0 - exponent) * log);
}

double HIntegralInverse(double value, double exponent) {
  return std::exp(value * Log1pOverX((1.0 - exponent) * value));
}

class KeySampler {
 public:
  explicit KeySampler(const Config& config)
      : entries_(config.entries),
        zipf_(config.distribution == "zipf"),
        exponent_(config.zipf_exponent),
        h_integral_x1_(HIntegral(1.5, exponent_) - 1.0),
        h_integral_entries_(HIntegral(entries_ + 0.5, exponent_)),
        acceptance_(2.0 - HIntegralInverse(
                                HIntegral(2.5, exponent_) - H(2.0, exponent_),
                                exponent_)) {}

  std::uint64_t Sample(std::uint64_t* random) const {
    if (!zipf_) {
      *random = XorShift(*random);
      return *random % entries_;
    }
    for (;;) {
      *random = XorShift(*random);
      const auto uniform = static_cast<double>(*random >> 11) *
                           (1.0 / 9007199254740992.0);
      const auto u = h_integral_entries_ +
                     uniform * (h_integral_x1_ - h_integral_entries_);
      const auto candidate = HIntegralInverse(u, exponent_);
      const auto rank = std::clamp<std::uint64_t>(
          static_cast<std::uint64_t>(std::floor(candidate + 0.5)), 1, entries_);
      if (static_cast<double>(rank) - candidate <= acceptance_ ||
          u >= HIntegral(rank + 0.5, exponent_) - H(rank, exponent_)) {
        return rank - 1;
      }
    }
  }

 private:
  std::uint64_t entries_;
  bool zipf_;
  double exponent_;
  double h_integral_x1_;
  double h_integral_entries_;
  double acceptance_;
};

struct Window {
  std::vector<std::uint64_t> reads;
  std::vector<std::uint64_t> writes;
  std::vector<std::uint64_t> deletes;
};

std::uint64_t ElapsedNanos(Clock::time_point start, Clock::time_point end) {
  return static_cast<std::uint64_t>(
      std::chrono::duration_cast<Nanoseconds>(end - start).count());
}

std::uint64_t Percentile(const std::vector<std::uint64_t>& sorted,
                         std::uint64_t permille) {
  if (sorted.empty()) {
    return 0;
  }
  const auto rank = (sorted.size() * permille + 999) / 1000;
  return sorted[std::max<std::size_t>(rank, 1) - 1];
}

void ReportLatency(const std::string& metric, std::uint64_t run,
                   std::vector<std::uint64_t> samples,
                   std::uint64_t elapsed_nanos) {
  if (samples.empty()) {
    return;
  }
  std::sort(samples.begin(), samples.end());
  const auto rate = static_cast<double>(samples.size()) * 1.0e9 /
                    static_cast<double>(elapsed_nanos);
  std::cout << metric << ',' << run << ',' << samples.size() << ','
            << elapsed_nanos << ',' << std::fixed << std::setprecision(3)
            << rate << ',' << Percentile(samples, 500) << ','
            << Percentile(samples, 950) << ',' << Percentile(samples, 990)
            << ',' << Percentile(samples, 999) << '\n';
}

void ReportThroughput(const std::string& metric, std::uint64_t run,
                      std::uint64_t operations, std::uint64_t elapsed_nanos) {
  const auto rate = static_cast<double>(operations) * 1.0e9 /
                    static_cast<double>(elapsed_nanos);
  std::cout << metric << ',' << run << ',' << operations << ',' << elapsed_nanos
            << ',' << std::fixed << std::setprecision(3) << rate << ",,,,\n";
}

void ReportWindow(const std::string& prefix, std::uint64_t index,
                  Window window, std::uint64_t elapsed_nanos) {
  const auto operations =
      window.reads.size() + window.writes.size() + window.deletes.size();
  ReportLatency(prefix + "_read", index, std::move(window.reads), elapsed_nanos);
  ReportLatency(prefix + "_overwrite", index, std::move(window.writes),
                elapsed_nanos);
  ReportLatency(prefix + "_delete", index, std::move(window.deletes),
                elapsed_nanos);
  ReportThroughput(prefix + "_all", index, operations, elapsed_nanos);
}

rocksdb::Options Options(
    const Config& config, bool create,
    const std::shared_ptr<rocksdb::Statistics>& statistics) {
  rocksdb::BlockBasedTableOptions table;
  table.block_cache = rocksdb::NewLRUCache(config.cache_bytes);
  rocksdb::Options options;
  options.create_if_missing = create;
  options.compression = rocksdb::kNoCompression;
  options.write_buffer_size = config.write_buffer_bytes;
  options.statistics = statistics;
  options.table_factory.reset(rocksdb::NewBlockBasedTableFactory(table));
  return options;
}

std::shared_ptr<rocksdb::Statistics> WalStatistics(const Config& config) {
  if (!config.report_wal_statistics) {
    return nullptr;
  }
  return rocksdb::CreateDBStatistics();
}

std::unique_ptr<rocksdb::DB> Open(
    const Config& config, bool create,
    const std::shared_ptr<rocksdb::Statistics>& statistics) {
  std::unique_ptr<rocksdb::DB> database;
  Require(rocksdb::DB::Open(Options(config, create, statistics),
                            config.database.string(), &database),
          "open");
  return database;
}

void ReportWalStatistics(
    std::string_view phase,
    const std::shared_ptr<rocksdb::Statistics>& statistics) {
  if (statistics == nullptr) {
    return;
  }
  rocksdb::HistogramData sync{};
  statistics->histogramData(rocksdb::WAL_FILE_SYNC_MICROS, &sync);
  std::cerr << "rocksdb_wal_stats phase=" << phase
            << " syncs="
            << statistics->getTickerCount(rocksdb::WAL_FILE_SYNCED)
            << " bytes="
            << statistics->getTickerCount(rocksdb::WAL_FILE_BYTES)
            << " histogram_count=" << sync.count
            << " sync_sum_micros=" << sync.sum
            << " sync_average_micros=" << sync.average
            << " sync_p50_micros=" << sync.median
            << " sync_p95_micros=" << sync.percentile95
            << " sync_p99_micros=" << sync.percentile99
            << " sync_max_micros=" << sync.max << '\n';
}

std::uint64_t PreloadCache(rocksdb::DB* database) {
  std::unique_ptr<rocksdb::Iterator> iterator(
      database->NewIterator(rocksdb::ReadOptions()));
  std::uint64_t count = 0;
  std::uint64_t value_bytes = 0;
  for (iterator->SeekToFirst(); iterator->Valid(); iterator->Next()) {
    if (count == std::numeric_limits<std::uint64_t>::max()) {
      Fail("entry count overflow");
    }
    const auto size = iterator->value().size();
    if (size > std::numeric_limits<std::uint64_t>::max() - value_bytes) {
      Fail("preload value byte count overflow");
    }
    value_bytes += size;
    ++count;
  }
  Require(iterator->status(), "preload cache");
  std::cerr << "sustained_preload entries=" << count
            << " value_bytes=" << value_bytes << '\n';
  return count;
}

std::string Hex(const std::array<std::uint8_t, 32>& bytes) {
  constexpr std::string_view digits = "0123456789abcdef";
  std::string encoded;
  encoded.reserve(bytes.size() * 2);
  for (const auto byte : bytes) {
    encoded.push_back(digits[byte >> 4]);
    encoded.push_back(digits[byte & 0x0f]);
  }
  return encoded;
}

void UpdateU64(Sha256* hash, std::uint64_t value) {
  std::array<char, 8> encoded{};
  for (std::size_t offset = 0; offset < encoded.size(); ++offset) {
    encoded[offset] = static_cast<char>(
        value >> static_cast<unsigned>(offset * 8));
  }
  hash->Update(encoded.data(), encoded.size());
}

struct LogicalVerification {
  std::uint64_t entries;
  std::string logical_hash;
};

LogicalVerification VerifyDatabase(rocksdb::DB* database) {
  Require(database->VerifyChecksum(), "verify database checksums");
  Sha256 hash;
  constexpr std::string_view domain("WSLDB logical snapshot\0", 23);
  hash.Update(domain.data(), domain.size());
  std::unique_ptr<rocksdb::Iterator> iterator(
      database->NewIterator(rocksdb::ReadOptions()));
  std::uint64_t entries = 0;
  for (iterator->SeekToFirst(); iterator->Valid(); iterator->Next()) {
    if (entries == std::numeric_limits<std::uint64_t>::max()) {
      Fail("entry count overflow");
    }
    ++entries;
    const auto key = iterator->key();
    const auto value = iterator->value();
    UpdateU64(&hash, key.size());
    hash.Update(key.data(), key.size());
    UpdateU64(&hash, value.size());
    hash.Update(value.data(), value.size());
  }
  Require(iterator->status(), "iterate for logical verification");
  return LogicalVerification{entries, Hex(hash.Finalize())};
}

void VerifySha256Implementation() {
  Sha256 hash;
  hash.Update("abc", 3);
  if (Hex(hash.Finalize()) !=
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    Fail("internal SHA-256 self-test failed");
  }
}

void Drain(rocksdb::DB* database, const std::string& prefix) {
  rocksdb::WaitForCompactOptions options;
  options.flush = true;
  options.wait_for_purge = true;
  const auto started = Clock::now();
  Require(database->WaitForCompact(options), "flush and compaction drain");
  const auto elapsed = ElapsedNanos(started, Clock::now());
  ReportLatency(prefix + "_drain", 0, {elapsed}, elapsed);
}

void Load(const Config& config) {
  if (std::filesystem::exists(config.database)) {
    Fail("load mode requires a non-existing database path");
  }
  const auto statistics = WalStatistics(config);
  auto database = Open(config, true, statistics);
  rocksdb::WriteOptions write_options;
  write_options.sync = true;
  std::string value(config.value_bytes, '\0');
  const auto started = Clock::now();
  for (std::uint64_t first = 0; first < config.entries;
       first += kLoadBatch) {
    rocksdb::WriteBatch batch;
    const auto end = std::min(first + kLoadBatch, config.entries);
    for (auto index = first; index < end; ++index) {
      const auto key = Key(index);
      FillValue(index, &value);
      Require(batch.Put(rocksdb::Slice(key.data(), key.size()), value),
              "build load batch");
    }
    Require(database->Write(write_options, &batch), "durable load batch");
  }
  const auto elapsed = ElapsedNanos(started, Clock::now());
  ReportThroughput("setup_sequential_load", 0, config.entries, elapsed);
  Drain(database.get(), "setup");
  const auto verification = VerifyDatabase(database.get());
  if (verification.entries != config.entries) {
    Fail("load did not create the configured number of entries");
  }
  std::cerr << "load_complete entries=" << verification.entries
            << " logical_hash=" << verification.logical_hash << '\n';
  ReportWalStatistics("load", statistics);
}

void Run(const Config& config) {
  const auto statistics = WalStatistics(config);
  auto database = Open(config, false, statistics);
  if (PreloadCache(database.get()) != config.entries) {
    Fail("database entry count does not match sustained workload configuration");
  }
  rocksdb::WriteOptions write_options;
  write_options.sync = true;
  rocksdb::ReadOptions read_options;
  KeySampler sampler(config);
  std::string value(config.value_bytes, '\0');
  std::string read_value;
  std::uint64_t read_bytes = 0;
  std::uint64_t random = kSeed;
  std::uint64_t operations = 0;
  std::uint64_t window_index = 0;
  Window window;
  Window total;
  std::uint64_t active_nanos = 0;
  const std::string prefix = config.distribution == "zipf" ? "w6" : "w5";
  const auto started = Clock::now();
  const auto deadline = started + std::chrono::seconds(config.duration_seconds);
  auto window_started = started;

  while (Clock::now() < deadline &&
         (config.operation_limit == 0 || operations < config.operation_limit)) {
    const auto index = sampler.Sample(&random);
    random = XorShift(random);
    const auto selector = RotateLeft(random, 23) % 100;
    const auto key = Key(index);
    const rocksdb::Slice key_slice(key.data(), key.size());
    const auto operation_started = Clock::now();
    if (selector < 80) {
      read_value.clear();
      Require(database->Get(read_options, key_slice, &read_value), "point read");
      read_bytes += read_value.size();
      const auto elapsed = ElapsedNanos(operation_started, Clock::now());
      window.reads.push_back(elapsed);
      total.reads.push_back(elapsed);
    } else if (selector < 95) {
      FillValue(index ^ RotateLeft(operations, 17), &value);
      Require(database->Put(write_options, key_slice, value), "durable overwrite");
      const auto elapsed = ElapsedNanos(operation_started, Clock::now());
      window.writes.push_back(elapsed);
      total.writes.push_back(elapsed);
    } else {
      std::string existing_value;
      Require(database->Get(read_options, key_slice, &existing_value),
              "delete existence check");
      Require(database->Delete(write_options, key_slice), "durable delete");
      FillValue(index ^ RotateLeft(operations, 17), &value);
      Require(database->Put(write_options, key_slice, value), "durable recreate");
      const auto elapsed = ElapsedNanos(operation_started, Clock::now());
      window.deletes.push_back(elapsed);
      total.deletes.push_back(elapsed);
    }
    if (operations == std::numeric_limits<std::uint64_t>::max()) {
      Fail("operation count overflow");
    }
    ++operations;
    const auto now = Clock::now();
    if (now - window_started >= std::chrono::seconds(1) || now >= deadline ||
        operations == config.operation_limit) {
      ++window_index;
      const auto elapsed = ElapsedNanos(window_started, now);
      if (elapsed > std::numeric_limits<std::uint64_t>::max() - active_nanos) {
        Fail("active duration overflow");
      }
      active_nanos += elapsed;
      ReportWindow(prefix, window_index, std::move(window), elapsed);
      window = Window{};
      if (window_index % config.maintenance_seconds == 0) {
        const auto maintenance_started = Clock::now();
        const auto verification = VerifyDatabase(database.get());
        if (verification.entries != config.entries) {
          Fail("periodic verification found an unexpected entry count");
        }
        const auto elapsed = ElapsedNanos(maintenance_started, Clock::now());
        ReportLatency(prefix + "_maintenance", window_index, {elapsed}, elapsed);
        std::cerr << "sustained_maintenance=" << window_index
                  << " entries=" << verification.entries
                  << " logical_hash=" << verification.logical_hash << '\n';
      }
      // Keep reporting and maintenance outside the next active window.
      window_started = Clock::now();
    }
  }

  ReportWindow(prefix + "_total", 0, std::move(total), active_nanos);
  ReportWalStatistics("run", statistics);
  Drain(database.get(), prefix);
  const auto verification = VerifyDatabase(database.get());
  if (verification.entries != config.entries) {
    Fail("sustained workload did not preserve its fixed-size dataset");
  }
  std::cerr << "sustained_complete operations=" << operations
            << " entries=" << verification.entries
            << " logical_hash=" << verification.logical_hash
            << " read_bytes=" << read_bytes
            << " distribution=" << config.distribution
            << " zipf_exponent=" << config.zipf_exponent << '\n';
}

}  // namespace

int main(int argc, char** argv) {
  try {
    if (argc == 2 && std::string_view(argv[1]) == "--version") {
      std::cout << "rocksdb_sustained_bench version "
                << rocksdb::GetRocksVersionAsString() << '\n';
      return 0;
    }
    VerifySha256Implementation();
    const auto config = ParseConfig(argc, argv);
    std::cout << "metric,run,operations,elapsed_ns,ops_per_second,p50_ns,"
                 "p95_ns,p99_ns,p999_ns\n";
    if (config.mode == "load") {
      Load(config);
    } else {
      Run(config);
    }
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "error: " << error.what() << '\n';
    return 1;
  }
}
