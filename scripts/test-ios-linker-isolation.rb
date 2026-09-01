#!/usr/bin/env ruby
# frozen_string_literal: true

require "fileutils"
require "json"
require "open3"
require "shellwords"

ROOT = File.expand_path("..", __dir__)
IOS_DIR = File.join(ROOT, "mobile", "ios")
SOURCE_PROJECT = File.join(IOS_DIR, "Runner.xcodeproj")
PROJECT = File.join(IOS_DIR, ".Runner-linker-isolation-#{Process.pid}.xcodeproj")
PODS_SUPPORT = File.join(
  ROOT,
  "mobile",
  "ios",
  "Pods",
  "Target Support Files",
  "Pods-Runner"
)
CONFIGURATIONS = %w[Debug Release Profile].freeze

abort "Temporary project already exists: #{PROJECT}" if File.exist?(PROJECT)

FileUtils.cp_r(SOURCE_PROJECT, PROJECT)
at_exit { FileUtils.rm_rf(PROJECT) }

def build_settings(target, configuration)
  stdout, stderr, status = Open3.capture3(
    "xcodebuild",
    "-project",
    PROJECT,
    "-target",
    target,
    "-configuration",
    configuration,
    "-showBuildSettings",
    "-json"
  )

  unless status.success?
    warn stderr
    abort "xcodebuild failed for #{target} #{configuration}"
  end

  result = JSON.parse(stdout).find { |entry| entry.fetch("target") == target }
  abort "xcodebuild returned no settings for #{target} #{configuration}" unless result

  result.fetch("buildSettings")
rescue JSON::ParserError => error
  warn stderr
  abort "Could not parse build settings for #{target} #{configuration}: #{error.message}"
end

def linker_flags(settings)
  value = settings.fetch("OTHER_LDFLAGS", "")
  value.is_a?(Array) ? value.join(" ") : value.to_s
end

def expected_runner_flags(configuration)
  path = File.join(PODS_SUPPORT, "Pods-Runner.#{configuration.downcase}.xcconfig")
  abort "Missing CocoaPods support file: #{path}" unless File.file?(path)

  assignment = File.foreach(path).find { |line| line.start_with?("OTHER_LDFLAGS =") }
  abort "Missing OTHER_LDFLAGS in #{path}" unless assignment

  Shellwords.split(assignment.split("=", 2).fetch(1)).reject { |flag| flag == "$(inherited)" }
end

CONFIGURATIONS.each do |configuration|
  extension_flags = Shellwords.split(
    linker_flags(build_settings("NotificationService", configuration))
  )
  unless extension_flags.empty?
    abort(
      "NotificationService #{configuration} inherited linker flags: " \
      "#{extension_flags.join(" ")}"
    )
  end

  runner_flags = Shellwords.split(linker_flags(build_settings("Runner", configuration)))
  expected_flags = expected_runner_flags(configuration)
  unless runner_flags == expected_flags
    abort(
      "Runner #{configuration} did not retain its CocoaPods linker flags.\n" \
      "Expected: #{expected_flags.join(" ")}\n" \
      "Actual:   #{runner_flags.join(" ")}"
    )
  end

  puts "#{configuration}: NotificationService isolated; Runner CocoaPods flags retained"
end
